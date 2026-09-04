//! Transport validation for editable, schema-labelled Expression data.
//!
//! These records are deliberately public. Metadata ownership does not apply:
//! an authorized transform may replace or remove a record. The host validates
//! the envelope, while addons define and interpret each schema's fields.

use super::WitExpressionPublicData;
use skript_parser::ExpressionPublicData;
use std::collections::BTreeSet;

pub(super) fn validate(entries: &[WitExpressionPublicData]) -> Result<(), String> {
    let mut schemas = BTreeSet::new();
    for entry in entries {
        if entry.schema_id.is_empty()
            || entry.schema_id.trim() != entry.schema_id
            || entry.schema_id.chars().any(char::is_control)
        {
            return Err(
                "public data schema ID must be nonblank, unpadded, and control-free".to_owned(),
            );
        }
        if entry.schema_version == 0 {
            return Err("public data schema version must be at least 1".to_owned());
        }
        if !schemas.insert(&entry.schema_id) {
            return Err(format!(
                "public data schema {} is repeated",
                entry.schema_id
            ));
        }
        let value: &serde_json::value::RawValue =
            serde_json::from_str(&entry.json).map_err(|error| {
                format!("invalid public data JSON for {}: {error}", entry.schema_id)
            })?;
        if !value.get().starts_with('{') {
            return Err(format!(
                "public data for {} must be a JSON object",
                entry.schema_id
            ));
        }
    }
    Ok(())
}

pub(super) fn from_wit(
    entries: Vec<WitExpressionPublicData>,
) -> Result<Vec<ExpressionPublicData>, String> {
    validate(&entries)?;
    Ok(entries
        .into_iter()
        .map(|entry| ExpressionPublicData {
            schema_id: entry.schema_id,
            schema_version: entry.schema_version,
            json: entry.json,
        })
        .collect())
}

pub(super) fn to_wit(entries: &[ExpressionPublicData]) -> Vec<WitExpressionPublicData> {
    entries
        .iter()
        .map(|entry| WitExpressionPublicData {
            schema_id: entry.schema_id.clone(),
            schema_version: entry.schema_version,
            json: entry.json.clone(),
        })
        .collect()
}

pub(super) fn same(left: &[WitExpressionPublicData], right: &[WitExpressionPublicData]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.schema_id == right.schema_id
                && left.schema_version == right.schema_version
                && left.json == right.json
        })
}

pub(super) fn size(entries: &[WitExpressionPublicData]) -> usize {
    entries.iter().fold(0usize, |size, entry| {
        size.saturating_add(entry.schema_id.len())
            .saturating_add(entry.json.len())
            .saturating_add(4)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(schema_id: &str, schema_version: u32, json: &str) -> WitExpressionPublicData {
        WitExpressionPublicData {
            schema_id: schema_id.to_owned(),
            schema_version,
            json: json.to_owned(),
        }
    }

    #[test]
    fn accepts_open_schemas_without_interpreting_their_contents() {
        let entries = vec![data(
            "third-party.variable",
            3,
            r#"{"scope":"custom","name":"x"}"#,
        )];
        validate(&entries).unwrap();
        assert!(same(&entries, &to_wit(&from_wit(entries.clone()).unwrap())));
        assert!(size(&entries) > entries[0].json.len());
    }

    #[test]
    fn rejects_invalid_envelopes_and_duplicate_schema_ids() {
        for entry in [
            data("", 1, "{}"),
            data(" x", 1, "{}"),
            data("x\ny", 1, "{}"),
            data("x", 0, "{}"),
            data("x", 1, "[1]"),
            data("x", 1, "{"),
        ] {
            assert!(validate(&[entry]).is_err());
        }
        assert!(validate(&[data("x", 1, "{}"), data("x", 2, "{}")]).is_err());
        validate(&[data("x", 1, " \n {\"name\": \"money\"} \t")]).unwrap();
    }

    #[test]
    fn keeps_json_numbers_lossless_instead_of_interpreting_them_as_host_floats() {
        let json = r#"{"integer":123456789012345678901234567890,"decimal":0.123456789012345678901234567890}"#;
        let records = from_wit(vec![data("example.numeric", 1, json)]).unwrap();
        assert_eq!(records[0].json, json);
    }
}
