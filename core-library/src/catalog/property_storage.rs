//! Cache immutable implementation proofs, never current values or read effects.
use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
};

use super::{SourceRecord, declared_method_exists, pinned_runtime, registration_records};
use crate::nlaocs::skript_parser_addon::types::RegisteredExpressionPayload;

const PROPERTY_BASE: &str = "org.skriptlang.skript.lang.properties.PropertyBaseExpression";
const MAX_KEY_BYTES: usize = 4096;
static SOURCES: LazyLock<RwLock<HashMap<Key, bool>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Source {
    format: String,
    schema: u32,
    snapshot: String,
    digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    source: Source,
    definition: String,
    registration: String,
    class: String,
    index: u64,
    pattern: String,
}

impl Key {
    fn cacheable(&self) -> bool {
        [
            self.source.format.len(),
            self.source.snapshot.len(),
            self.source.digest.len(),
            self.definition.len(),
            self.registration.len(),
            self.class.len(),
            self.pattern.len(),
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .is_some_and(|bytes| bytes <= MAX_KEY_BYTES)
    }
}

/// The proof depends only on the immutable source and exact registration, not
/// on document revision, children, current variables or expected result types.
/// A component instance owns this cache; initialization still checks runtime.
pub(crate) fn standard_property_source(payload: &RegisteredExpressionPayload) -> bool {
    if !pinned_runtime() {
        return false;
    }
    let Some(source) = catalog_source() else {
        return false;
    };
    let key = Key {
        source,
        definition: payload.definition_id.clone(),
        registration: payload.registration_id.clone(),
        class: payload.element_class.clone(),
        index: payload.pattern_index,
        pattern: payload.pattern.clone(),
    };
    cached(&SOURCES, &key, || {
        let records = registration_records(&key.registration).ok()?;
        certify_source(
            &key,
            &records,
            |class| {
                declared_method_exists(
                    class,
                    "beforeChange",
                    &["ch.njol.skript.lang.Expression", "java.lang.Object[]"],
                    None,
                )
                .ok()
                .flatten()
            },
            catalog_source,
        )
    })
    .unwrap_or(false)
}

fn cached(
    cache: &RwLock<HashMap<Key, bool>>,
    key: &Key,
    resolve: impl FnOnce() -> Option<bool>,
) -> Option<bool> {
    if !key.cacheable() {
        return resolve();
    }
    if let Some(result) = cache
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .copied()
    {
        return Some(result);
    }
    // No lock spans host calls, which can fail or re-enter other guest helpers.
    let result = resolve()?;
    let mut cache = cache
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= super::MAX_CACHED_CHANGE_CONTRACTS && !cache.contains_key(key) {
        cache.clear();
    }
    cache.insert(key.clone(), result);
    Some(result)
}

fn certify_source(
    key: &Key,
    records: &[SourceRecord],
    has_before_change: impl FnMut(&str) -> Option<bool>,
    current_source: impl FnOnce() -> Option<Source>,
) -> Option<bool> {
    let result = from_records(key, records, has_before_change)?;
    // Fence both positive and definitive negative results. Failed reads or
    // unavailable method metadata are not a reusable negative certificate.
    (current_source()? == key.source).then_some(result)
}

fn from_records(
    key: &Key,
    records: &[SourceRecord],
    mut has_before_change: impl FnMut(&str) -> Option<bool>,
) -> Option<bool> {
    let mut expressions = records.iter().filter(|record| record.document == "Expressions.json");
    let Some(record) = expressions.next() else {
        return Some(false);
    };
    if expressions.next().is_some() {
        return Some(false);
    }
    let record: serde_json::Value = serde_json::from_slice(&record.bytes).ok()?;
    let Ok(index) = usize::try_from(key.index) else {
        return Some(false);
    };
    if !record.is_object()
        || ["kind", "definitionId", "registrationId", "elementClass", "superClass"]
            .iter()
            .any(|field| record[*field].as_str().is_none())
        || record["addon"]["name"].as_str().is_none()
        || record["addon"]["version"].as_str().is_none()
    {
        return None;
    }
    let patterns = record["patterns"].as_array()?;
    if patterns.get(index).is_some_and(|pattern| !pattern.is_string()) {
        return None;
    }
    // Reject other registrations and non-property implementations before any
    // method query, using the same source authentication as other syntax kinds.
    if !super::standard_syntax_record(
        &record,
        "expression",
        &key.definition,
        &key.registration,
        &key.class,
        index,
        Some(&key.pattern),
    ) || record["superClass"].as_str() != Some(PROPERTY_BASE)
    {
        return Some(false);
    }
    for class in [
        key.class.as_str(),
        PROPERTY_BASE,
        "ch.njol.skript.lang.util.SimpleExpression",
    ] {
        if has_before_change(class)? {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(target_arch = "wasm32")]
fn catalog_source() -> Option<Source> {
    let source = crate::nlaocs::skript_parser_addon::catalog_data::source().ok()??;
    Some(Source {
        format: source.format,
        schema: source.schema_version,
        snapshot: source.snapshot_id,
        digest: source.source_digest,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn catalog_source() -> Option<Source> {
    None
}

#[cfg(test)]
mod tests;
