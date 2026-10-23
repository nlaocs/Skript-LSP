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

use crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption;

pub(super) fn preserves_bounds(classes: &[String], options: &[ExpressionTypeOption]) -> bool {
    if classes.is_empty() || classes.len() > 64 {
        return false;
    }
    let immutable = crate::catalog::identity_delta_classes();
    classes
        .iter()
        .all(|class| immutable.contains(class) || (class == COMPONENT && component_family(options)))
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

fn registry_headers(
    options: &[ExpressionTypeOption],
    digest: &str,
    snapshot: &str,
) -> Option<Vec<Value>> {
    if options.is_empty() || options.len() > 1024 {
        return None;
    }
    let mut indices = std::collections::BTreeSet::new();
    let mut headers = Vec::with_capacity(options.len());
    for option in options {
        let record = option.source_record.as_ref()?;
        if record.source_digest != digest
            || record.snapshot_id != snapshot
            || record.document != "Types.json"
            || record.index >= options.len() as u64
            || !indices.insert(record.index)
        {
            return None;
        }
        headers.push(serde_json::json!({
            "typeParseOrder": option.type_parse_order,
            "originalClass": option.class_name,
            "addon": {"name": option.addon_name, "version": option.addon_version}
        }));
    }
    (stock_headers(&headers)
        && headers
            .iter()
            .filter(|header| header["originalClass"].as_str() == Some(COMPONENT))
            .count()
            == 1)
        .then_some(headers)
}

#[cfg(target_arch = "wasm32")]
fn component_family(options: &[ExpressionTypeOption]) -> bool {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    if !crate::runtime::skript_version_is("2.15.4") {
        return false;
    }
    let Some(source) = catalog_data::source().ok().flatten() else {
        return false;
    };
    // This context is the complete normalized Catalog::types list. SSG copies
    // required owner/class/order fields directly from every raw Type record.
    // Completeness comes from the host's all-options contract, not from dense
    // indices alone (a contiguous prefix is not a complete registry). Indices
    // and source identities check its shape/provenance. No large literal tables
    // are read; unchecked native Catalog constructors remain caller-validated.
    let Some(mut headers) = registry_headers(options, &source.source_digest, &source.snapshot_id)
    else {
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
    let result = read_component_record(options, &source.source_digest, &source.snapshot_id)
        .is_some_and(|record| {
            let matching = headers
                .iter()
                .position(|header| header["originalClass"].as_str() == Some(COMPONENT));
            let Some(index) = matching else {
                return false;
            };
            if record["typeParseOrder"] != headers[index]["typeParseOrder"]
                || record["originalClass"] != headers[index]["originalClass"]
                || record["addon"] != headers[index]["addon"]
            {
                return false;
            }
            headers[index] = record;
            component_registry(&headers)
        })
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
fn component_family(_options: &[ExpressionTypeOption]) -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
fn read_component_record(
    options: &[ExpressionTypeOption],
    digest: &str,
    snapshot: &str,
) -> Option<Value> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    let option = options
        .iter()
        .find(|option| option.class_name == COMPONENT)?;
    let record = option.source_record.as_ref()?;
    const MAX_BYTES: u32 = 64 * 1024;
    if record.byte_length == 0 || record.byte_length > u64::from(MAX_BYTES) {
        return None;
    }
    let chunk =
        catalog_data::read_record(digest, snapshot, "Types.json", record.index, 0, MAX_BYTES)
            .ok()??;
    if chunk.offset != 0
        || chunk.total_length != record.byte_length
        || chunk.bytes.len() as u64 != chunk.total_length
    {
        return None;
    }
    serde_json::from_slice(&chunk.bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn option(index: u64, class: &str) -> ExpressionTypeOption {
        ExpressionTypeOption {
            source_record: Some(
                crate::nlaocs::skript_parser_addon::types::CatalogRecordRef {
                    source_digest: "digest".into(),
                    snapshot_id: "snapshot".into(),
                    document: "Types.json".into(),
                    index,
                    byte_length: 100,
                },
            ),
            definition_id: format!("type:{index}"),
            registration_id: format!("type:{index}"),
            addon_name: "Skript".into(),
            addon_version: "2.15.4".into(),
            code_name: format!("type{index}"),
            class_name: class.into(),
            parser_class: None,
            type_parse_order: index,
            before: Vec::new(),
            after: Vec::new(),
            singular: String::new(),
            plural: String::new(),
            user_input_patterns: Vec::new(),
            has_parser: false,
            parse_contexts: Vec::new(),
            has_supplier: false,
            default_expression: None,
        }
    }

    #[test]
    fn storage_registry_context_requires_complete_same_source_headers() {
        let options = vec![option(0, "java.lang.String"), option(1, COMPONENT)];
        assert!(registry_headers(&options, "digest", "snapshot").is_some());
        for mutation in [
            "missing-source",
            "source",
            "snapshot",
            "document",
            "duplicate-index",
            "index-gap",
            "order",
            "addon",
            "duplicate-component",
        ] {
            let mut changed = options.clone();
            match mutation {
                "missing-source" => changed[1].source_record = None,
                "source" => {
                    changed[1].source_record.as_mut().unwrap().source_digest = "other".into()
                }
                "snapshot" => {
                    changed[1].source_record.as_mut().unwrap().snapshot_id = "other".into()
                }
                "document" => {
                    changed[1].source_record.as_mut().unwrap().document = "Other.json".into()
                }
                "duplicate-index" => changed[1].source_record.as_mut().unwrap().index = 0,
                "index-gap" => changed[1].source_record.as_mut().unwrap().index = 2,
                "order" => changed[1].type_parse_order = 0,
                "addon" => changed[0].addon_name = "Addon".into(),
                _ => changed[0].class_name = COMPONENT.into(),
            }
            assert!(
                registry_headers(&changed, "digest", "snapshot").is_none(),
                "{mutation}"
            );
        }
        assert!(registry_headers(&[], "digest", "snapshot").is_none());
        assert!(registry_headers(&vec![options[0].clone(); 1025], "digest", "snapshot").is_none());
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
