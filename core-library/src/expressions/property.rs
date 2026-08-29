use super::{SemanticResolution, metadata, metadata_value};
use crate::catalog;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredExpressionPropertyOption,
};

pub(super) fn resolve(payload: &RegisteredExpressionPayload, mode: &str) -> SemanticResolution {
    let options = match selected_options(payload) {
        Ok(options) => options,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let source = source_child_for_options(payload, &options);
    resolve_options(
        &payload.registration_id,
        &options,
        source,
        source
            .and_then(|child| child.multiplicity)
            .unwrap_or(DynamicMultiplicity::Both),
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
    let options = match selected_options(payload) {
        Ok(options) => options,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let source = source_child_for_options(payload, &options).or(Some(source));
    resolve_options(
        &payload.registration_id,
        &options,
        source,
        source
            .and_then(|child| child.multiplicity)
            .unwrap_or(DynamicMultiplicity::Both),
        &format!("{mode}-property"),
    )
}

pub(super) fn source_child_for_options<'a>(
    payload: &'a RegisteredExpressionPayload,
    options: &[RegisteredExpressionPropertyOption],
) -> Option<&'a RegisteredExpressionChild> {
    let index = options.first()?.source_child_index;
    debug_assert!(
        options
            .iter()
            .all(|option| option.source_child_index == index)
    );
    usize::try_from(index)
        .ok()
        .and_then(|index| payload.children.get(index))
}

pub(super) fn selected_options(
    payload: &RegisteredExpressionPayload,
) -> Result<Vec<RegisteredExpressionPropertyOption>, String> {
    let options = if payload.selected_property_option_indices.is_empty() {
        payload.property_options.clone()
    } else {
        payload
            .selected_property_option_indices
            .iter()
            .map(|index| {
                usize::try_from(*index)
                    .ok()
                    .and_then(|index| payload.property_options.get(index))
                    .cloned()
                    .ok_or_else(|| format!("selected Property option index {index} is invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if let Some(first) = options.first()
        && options
            .iter()
            .any(|option| option.source_child_index != first.source_child_index)
    {
        return Err(
            "matching Property options refer to different source Expressions; a WASM policy must select one source"
                .to_owned(),
        );
    }
    Ok(options)
}

pub(super) fn resolve_options(
    subject_id: &str,
    options: &[RegisteredExpressionPropertyOption],
    source: Option<&RegisteredExpressionChild>,
    multiplicity: DynamicMultiplicity,
    mode: &str,
) -> SemanticResolution {
    if options.is_empty() {
        return SemanticResolution::Reject(
            "source type has no matching registered property".to_owned(),
        );
    }
    let property_registration_id = &options[0].property_registration_id;
    let property_source_index = options[0].property_source_index;
    if options.iter().any(|option| {
        option.property_registration_id != *property_registration_id
            || option.property_source_index != property_source_index
    }) {
        return SemanticResolution::Reject(
            "multiple Property registrations match this Expression; a version-specific WASM policy must select one"
                .to_owned(),
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
    let mut metadata = vec![metadata("semantic-mode", mode)];
    metadata.push(
        catalog::change_contract_metadata(
            subject_id,
            &catalog::property_change_contract(options, source),
        )
        .expect("an in-memory change contract must serialize"),
    );
    SemanticResolution::Resolved {
        return_type,
        multiplicity,
        metadata,
    }
}

pub(super) fn source_child(
    payload: &RegisteredExpressionPayload,
) -> Option<&RegisteredExpressionChild> {
    payload.children.iter().find(|child| {
        metadata_value(&child.metadata, "semantic-role") != Some("target-type")
            && metadata_value(&child.metadata, "target-class").is_none()
    })
}

pub(super) fn source_multiplicity(payload: &RegisteredExpressionPayload) -> DynamicMultiplicity {
    source_child(payload)
        .and_then(|child| child.multiplicity)
        .unwrap_or(DynamicMultiplicity::Both)
}
