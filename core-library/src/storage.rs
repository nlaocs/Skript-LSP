//! Provider-owned facts about a value AFTER beforeChange, clone and storage
//! conversion. These do not certify the evaluation of the original expression.
use serde::{Deserialize, Serialize};

use crate::nlaocs::skript_parser_addon::types::{MetadataEntry, ParseSummary};

pub(crate) const METADATA_KEY: &str = "skript.semantic.storage-result";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResultType {
    pub version: u32,
    pub scope: String,
    pub types: Vec<String>,
    pub complete: bool,
    pub multiple: bool,
    pub maybe_unset: bool,
    pub preserves_local_bindings: bool,
}

impl ResultType {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.scope == "local-scalar"
            && self.complete
            && !self.multiple
            && !self.types.is_empty()
            && self.types.len() <= 64
            && self
                .types
                .iter()
                .all(|ty| !ty.is_empty() && ty != "java.lang.Object")
            && self
                .types
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.types.len()
    }

    pub fn metadata(self) -> MetadataEntry {
        MetadataEntry {
            key: METADATA_KEY.into(),
            value: serde_json::to_string(&self).expect("storage facts serialize"),
            owner_component_id: None,
        }
    }
}

pub(crate) fn for_summary(summary: &ParseSummary) -> Option<ResultType> {
    let mut entries = summary
        .metadata
        .iter()
        .filter(|entry| entry.key == METADATA_KEY);
    let entry = entries.next()?;
    if entries.next().is_some() || entry.owner_component_id.as_deref() != Some(crate::COMPONENT_ID)
    {
        return None;
    }
    let result: ResultType = serde_json::from_str(&entry.value).ok()?;
    result.valid().then_some(result)
}

pub(crate) fn for_local_set(
    summary: Option<&ParseSummary>,
    is_set: bool,
    target_is_list: Option<bool>,
) -> Option<ResultType> {
    (is_set && target_is_list == Some(false))
        .then(|| summary.and_then(for_summary))
        .flatten()
}

/// Called after final semantic resolution, including any explicit property
/// target conversion. Never copy an earlier property's pre-conversion type.
pub(crate) fn annotate_property(
    payload: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionPayload,
    resolution: &mut crate::expressions::SemanticResolution,
) {
    use crate::expressions::SemanticResolution;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState,
    };
    let SemanticResolution::Resolved {
        possible_return_types,
        possible_return_types_state,
        multiplicity,
        metadata,
        ..
    } = resolution
    else {
        return;
    };
    if *possible_return_types_state != ExpressionPossibleReturnTypesState::Complete
        || *multiplicity != DynamicMultiplicity::Single
        || !crate::catalog::standard_property_source(payload)
        || !crate::types::storage_preserves_bounds(possible_return_types, &payload.type_options)
        || metadata.iter().any(|entry| entry.key == METADATA_KEY)
    {
        return;
    }
    metadata.push(
        ResultType {
            version: 1,
            scope: "local-scalar".into(),
            types: possible_return_types.clone(),
            complete: true,
            multiple: false,
            maybe_unset: true,
            preserves_local_bindings: true,
        }
        .metadata(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_certificate_requires_complete_scalar_local_facts() {
        let result = ResultType {
            version: 1,
            scope: "local-scalar".into(),
            types: vec!["fixture.Stored".into()],
            complete: true,
            multiple: false,
            maybe_unset: true,
            preserves_local_bindings: false,
        };
        assert!(result.valid());
        for mutation in [
            "version",
            "scope",
            "types",
            "complete",
            "multiple",
            "duplicate",
        ] {
            let mut changed = result.clone();
            match mutation {
                "version" => changed.version = 2,
                "scope" => changed.scope = "global".into(),
                "types" => changed.types = vec!["java.lang.Object".into()],
                "complete" => changed.complete = false,
                "multiple" => changed.multiple = true,
                _ => changed.types.push(changed.types[0].clone()),
            }
            assert!(!changed.valid(), "{mutation}");
        }
    }

    #[test]
    fn storage_input_owner_version_and_uniqueness_are_required() {
        use crate::nlaocs::skript_parser_addon::types::{
            DynamicMultiplicity, ExpressionPossibleReturnTypesState,
        };
        let result = ResultType {
            version: 1,
            scope: "local-scalar".into(),
            types: vec!["fixture.Stored".into()],
            complete: true,
            multiple: false,
            maybe_unset: true,
            preserves_local_bindings: false,
        };
        let mut entry = result.metadata();
        entry.owner_component_id = Some(crate::COMPONENT_ID.into());
        let mut summary = ParseSummary {
            kind: "expression".into(),
            definition_id: None,
            registration_id: None,
            element_class: None,
            pattern_index: None,
            return_type: Some("fixture.Input".into()),
            possible_return_types: vec!["fixture.Input".into()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            default_expression: None,
            public_data: Vec::new(),
            metadata: vec![entry.clone()],
        };
        let stored = for_summary(&summary).unwrap();
        assert_eq!(stored.types, ["fixture.Stored"]);
        assert!(!stored.preserves_local_bindings);
        assert!(for_local_set(Some(&summary), true, Some(false)).is_some());
        for (is_set, target_is_list) in [(false, Some(false)), (true, Some(true)), (true, None)] {
            assert!(for_local_set(Some(&summary), is_set, target_is_list).is_none());
        }
        assert!(for_local_set(None, true, Some(false)).is_none());
        summary.metadata.push(entry.clone());
        assert!(for_summary(&summary).is_none());
        summary.metadata = vec![entry.clone()];
        summary.metadata[0].owner_component_id = Some("unrelated.provider".into());
        assert!(for_summary(&summary).is_none());
        summary.metadata = vec![entry];
        summary.metadata[0].value = "{broken".into();
        assert!(for_summary(&summary).is_none());
    }
}
