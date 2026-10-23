//! Source-bound standard value-family guarantees, separate from literal parsing.
//! Unknown providers, overrides and storage callbacks never inherit these facts.
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
};

const COMPONENT: &str = "net.kyori.adventure.text.Component";
static FAMILIES: LazyLock<RwLock<HashMap<String, bool>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

use crate::nlaocs::skript_parser_addon::catalog_data::CatalogFieldPage;

pub(super) fn preserves_bounds(classes: &[String]) -> bool {
    if classes.is_empty() || classes.len() > 64 {
        return false;
    }
    let immutable = crate::catalog::identity_delta_classes();
    classes
        .iter()
        .all(|class| immutable.contains(class) || (class == COMPONENT && component_family()))
}

fn stock_headers(records: &[Value]) -> bool {
    // Adventure Component is an immutable, NonExtendable public API family.
    // That API guarantee closes the value domain; an interface return bound or
    // ClassHierarchy alone does NOT. Existing implementation classes can still
    // be shadowed by addon ClassInfo, so this initial policy certifies only a
    // complete stock registry. Custom Component implementations are API-external.
    !records.is_empty()
        && records.len() <= 1024
        && records.iter().enumerate().all(|(index, record)| {
            record["typeParseOrder"].as_u64() == Some(index as u64)
                && record["originalClass"]
                    .as_str()
                    .is_some_and(|class| !class.is_empty())
                && record["addon"]["name"].as_str() == Some("Skript")
                && record["addon"]["version"].as_str() == Some("2.15.4")
        })
}

fn component_registry(records: &[Value]) -> bool {
    if !stock_headers(records) {
        return false;
    }
    let matches = records
        .iter()
        .filter(|record| record["originalClass"].as_str() == Some(COMPONENT))
        .collect::<Vec<_>>();
    matches.len() == 1
        && matches[0]["typeRuntime"]["classInfoClass"].as_str()
            == Some("org.skriptlang.skript.bukkit.text.types.TextComponentClassInfo")
        && crate::catalog::source_clone_identity(matches[0])
}

// The complete page sequence is authoritative. A dense prefix alone is not.
fn append_headers(records: &mut Vec<Value>, page: &CatalogFieldPage) -> Option<bool> {
    let before = records.len();
    if page.items.len() > 256 || before.checked_add(page.items.len())? > 1024 {
        return None;
    }
    for item in &page.items {
        if item.index != records.len() as u64 {
            return None;
        }
        let value: Value = serde_json::from_slice(&item.json).ok()?;
        if !value.is_object() {
            return None;
        }
        records.push(value);
    }
    if !records.is_empty() && !stock_headers(records) {
        return None;
    }
    match page.next_offset {
        None => Some(true),
        Some(next) if next == records.len() as u64 && next > before as u64 => Some(false),
        _ => None,
    }
}

#[cfg(target_arch = "wasm32")]
fn component_family() -> bool {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    if !crate::runtime::skript_version_is("2.15.4") {
        return false;
    }
    let Some(source) = catalog_data::source().ok().flatten() else {
        return false;
    };
    if let Some(result) = FAMILIES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&source.source_digest)
        .copied()
    {
        return result;
    }
    let result = (|| {
        let mut headers = read_headers(&source.source_digest, &source.snapshot_id)?;
        let matches = headers
            .iter()
            .enumerate()
            .filter(|(_, value)| value["originalClass"].as_str() == Some(COMPONENT))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return None;
        }
        let index = matches[0];
        let record =
            read_component_record(index as u64, &source.source_digest, &source.snapshot_id)?;
        for field in ["typeParseOrder", "originalClass", "addon"] {
            if record[field] != headers[index][field] {
                return None;
            }
        }
        headers[index] = record;
        Some(component_registry(&headers))
    })()
    .unwrap_or(false)
        && catalog_data::source().ok().flatten().is_some_and(|after| {
            after.source_digest == source.source_digest && after.snapshot_id == source.snapshot_id
        });
    let mut cache = FAMILIES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= 16 {
        cache.clear();
    }
    cache.insert(source.source_digest, result);
    result
}
#[cfg(not(target_arch = "wasm32"))]
fn component_family() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
fn read_headers(digest: &str, snapshot: &str) -> Option<Vec<Value>> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    let fields = ["typeParseOrder", "originalClass", "addon"].map(str::to_owned);
    let mut records = Vec::new();
    loop {
        let page = catalog_data::read_record_fields(
            digest,
            snapshot,
            "Types.json",
            &fields,
            records.len() as u64,
            256,
        )
        .ok()??;
        if append_headers(&mut records, &page)? {
            return Some(records);
        }
    }
}
#[cfg(target_arch = "wasm32")]
fn read_component_record(index: u64, digest: &str, snapshot: &str) -> Option<Value> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    const MAX_BYTES: u32 = 64 * 1024;
    let chunk =
        catalog_data::read_record(digest, snapshot, "Types.json", index, 0, MAX_BYTES).ok()??;
    if chunk.offset != 0
        || chunk.total_length == 0
        || chunk.total_length > u64::from(MAX_BYTES)
        || chunk.bytes.len() as u64 != chunk.total_length
    {
        return None;
    }
    serde_json::from_slice(&chunk.bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_registry_projection_requires_contiguous_complete_pages() {
        use crate::nlaocs::skript_parser_addon::catalog_data::CatalogFieldRecord;
        let record = |index| {
            CatalogFieldRecord { index, json: serde_json::to_vec(&serde_json::json!({"typeParseOrder":index,"originalClass":"fixture.Type","addon":{"name":"Skript","version":"2.15.4"}})).unwrap() }
        };
        let mut records = Vec::new();
        assert_eq!(
            append_headers(
                &mut records,
                &CatalogFieldPage {
                    items: vec![record(0)],
                    next_offset: Some(1)
                }
            ),
            Some(false)
        );
        assert_eq!(
            append_headers(
                &mut records,
                &CatalogFieldPage {
                    items: vec![record(1)],
                    next_offset: None
                }
            ),
            Some(true)
        );
        for mutation in [
            "empty-progress",
            "index-gap",
            "duplicate-index",
            "next-gap",
            "malformed",
            "non-object",
            "missing-owner",
            "addon",
        ] {
            let mut page = CatalogFieldPage {
                items: vec![record(0)],
                next_offset: None,
            };
            match mutation {
                "empty-progress" => {
                    page.items.clear();
                    page.next_offset = Some(0);
                }
                "index-gap" => page.items[0].index = 1,
                "duplicate-index" => page.items.push(record(0)),
                "next-gap" => page.next_offset = Some(2),
                "malformed" => page.items[0].json = b"{broken".to_vec(),
                "non-object" => page.items[0].json = b"null".to_vec(),
                "missing-owner" => page.items[0].json = b"{}".to_vec(),
                _ => {
                    let mut value: Value = serde_json::from_slice(&page.items[0].json).unwrap();
                    value["addon"]["name"] = Value::from("Addon");
                    page.items[0].json = serde_json::to_vec(&value).unwrap();
                }
            }
            assert!(
                append_headers(&mut Vec::new(), &page).is_none(),
                "{mutation}"
            );
        }
        assert!(
            append_headers(
                &mut Vec::new(),
                &CatalogFieldPage {
                    items: (0..257).map(record).collect(),
                    next_offset: None
                }
            )
            .is_none()
        );
    }

    #[test]
    fn standard_component_storage_needs_full_unshadowed_dispatch_evidence() {
        let records: Vec<Value> = serde_json::from_str(include_str!("../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-type-runtime/Types.json")).unwrap();
        assert!(component_registry(&records));
        let index = records
            .iter()
            .position(|record| record["originalClass"] == COMPONENT)
            .unwrap();
        let mut projected = records.iter().map(|record| serde_json::json!({
            "typeParseOrder": record["typeParseOrder"], "originalClass": record["originalClass"], "addon": record["addon"]
        })).collect::<Vec<_>>();
        projected[index] = records[index].clone();
        assert!(component_registry(&projected));
        assert!(
            projected
                .iter()
                .all(|record| record.get("registeredLiteralPatterns").is_none())
        );
        assert!(!component_registry(&[serde_json::json!({"unknown":true})]));
        for field in ["typeRuntime", "addon", "typeParseOrder"] {
            let mut changed = records.clone();
            changed[index].as_object_mut().unwrap().remove(field);
            assert!(!component_registry(&changed), "{field}");
        }
        for (field, value) in [
            ("clonerState", Value::from("present")),
            ("serializeAsState", Value::from("present")),
            ("cloneMethodDeclaringClass", Value::from("addon.Override")),
            ("cloneMethodUnknownReason", Value::from("failed")),
        ] {
            let mut changed = records.clone();
            changed[index]["typeRuntime"][field] = value;
            assert!(!component_registry(&changed), "{field}");
        }
        let mut changed = records.clone();
        changed[0]["addon"]["name"] = Value::from("Addon");
        assert!(
            !component_registry(&changed),
            "unrelated addon Type also disables the stock-only policy"
        );
        let mut changed = records.clone();
        changed.push(records[index].clone());
        assert!(!component_registry(&changed));
        assert!(!component_registry(&[]));
    }
}
