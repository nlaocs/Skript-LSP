use super::{SemanticResolution, metadata, metadata_value};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredExpressionPropertyOption,
};

pub(super) fn resolve(payload: &RegisteredExpressionPayload, mode: &str) -> SemanticResolution {
    resolve_options(
        &payload.property_options,
        source_multiplicity(payload),
        mode,
    )
}

pub(super) fn resolve_count_or_property(
    payload: &RegisteredExpressionPayload,
    mode: &str,
) -> SemanticResolution {
    let Some(source) = source_child(payload) else {
        return SemanticResolution::Reject(
            "property Expression requires a source Expression".to_owned(),
        );
    };
    let uses_property = payload
        .tags
        .iter()
        .any(|tag| tag.value == "s" && !tag.implicit)
        || matches!(source.multiplicity, Some(DynamicMultiplicity::Single));
    if !uses_property {
        return super::resolved(
            "java.lang.Long",
            DynamicMultiplicity::Single,
            &format!("{mode}-count"),
        );
    }
    resolve_options(
        &payload.property_options,
        source.multiplicity.unwrap_or(DynamicMultiplicity::Both),
        &format!("{mode}-property"),
    )
}

pub(super) fn resolve_options(
    options: &[RegisteredExpressionPropertyOption],
    multiplicity: DynamicMultiplicity,
    mode: &str,
) -> SemanticResolution {
    if options.is_empty() {
        return SemanticResolution::Reject(
            "source type has no matching registered property".to_owned(),
        );
    }
    let mut return_types = options
        .iter()
        .flat_map(|option| option.return_types.iter().cloned())
        .collect::<Vec<_>>();
    return_types.sort();
    return_types.dedup();
    let return_type = match return_types.as_slice() {
        [] => {
            return SemanticResolution::Reject(
                "matching property handlers have no return type".to_owned(),
            );
        }
        [only] => only.clone(),
        _ => "java.lang.Object".to_owned(),
    };
    SemanticResolution::Resolved {
        return_type,
        multiplicity,
        metadata: vec![metadata("semantic-mode", mode)],
    }
}

pub(super) fn source_child(
    payload: &RegisteredExpressionPayload,
) -> Option<&RegisteredExpressionChild> {
    payload
        .children
        .iter()
        .find(|child| metadata_value(&child.metadata, "target-class").is_none())
}

pub(super) fn source_multiplicity(payload: &RegisteredExpressionPayload) -> DynamicMultiplicity {
    source_child(payload)
        .and_then(|child| child.multiplicity)
        .unwrap_or(DynamicMultiplicity::Both)
}
