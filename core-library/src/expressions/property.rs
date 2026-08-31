use super::{SemanticResolution, metadata, metadata_value, resolved_with_possible_types};
use crate::catalog;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
    RegisteredExpressionPayload, RegisteredExpressionPropertyOption,
};

pub(super) fn resolve(payload: &RegisteredExpressionPayload, mode: &str) -> SemanticResolution {
    let options = match selected_options(payload) {
        Ok(options) => options,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let source = source_child_for_options(payload, &options);
    if !options.is_empty() && source.is_none() {
        return SemanticResolution::Unresolved {
            reason: "matching property registration refers to an unavailable source Expression"
                .to_owned(),
            metadata: vec![metadata("semantic-mode", mode)],
        };
    }
    let Some(multiplicity) = source.and_then(|child| child.multiplicity) else {
        if options.is_empty() {
            return resolve_options(
                &payload.registration_id,
                &options,
                source,
                DynamicMultiplicity::Single,
                mode,
            );
        }
        return SemanticResolution::Unresolved {
            reason: "property source multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", mode)],
        };
    };
    resolve_options(
        &payload.registration_id,
        &options,
        source,
        multiplicity,
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
    let explicit_property = payload
        .tags
        .iter()
        .any(|tag| tag.value == "s" && !tag.implicit);
    match (explicit_property, source.multiplicity) {
        (false, Some(DynamicMultiplicity::Multiple)) => {
            return super::resolved(
                "java.lang.Long",
                DynamicMultiplicity::Single,
                &format!("{mode}-count"),
            );
        }
        (true, _) | (false, Some(DynamicMultiplicity::Single)) => {}
        (false, Some(DynamicMultiplicity::Both)) => {
            return resolve_ambiguous_count_or_property(payload, source, mode);
        }
        (false, None) => {
            return SemanticResolution::Unresolved {
                reason: "cannot decide between property access and list counting because source multiplicity is unresolved".to_owned(),
                metadata: vec![metadata("semantic-mode", &format!("{mode}-count-or-property"))],
            };
        }
    }
    let options = match selected_options(payload) {
        Ok(options) => options,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let source = source_child_for_options(payload, &options).or(Some(source));
    let Some(multiplicity) = source.and_then(|child| child.multiplicity) else {
        return SemanticResolution::Unresolved {
            reason: "property source multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", &format!("{mode}-property"))],
        };
    };
    resolve_options(
        &payload.registration_id,
        &options,
        source,
        multiplicity,
        &format!("{mode}-property"),
    )
}

fn resolve_ambiguous_count_or_property(
    payload: &RegisteredExpressionPayload,
    source: &RegisteredExpressionChild,
    mode: &str,
) -> SemanticResolution {
    let options = match selected_options(payload) {
        Ok(options) => options,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let property_source = source_child_for_options(payload, &options).or(Some(source));
    let property = resolve_options(
        &payload.registration_id,
        &options,
        property_source,
        DynamicMultiplicity::Both,
        &format!("{mode}-property"),
    );
    let SemanticResolution::Resolved {
        possible_return_types,
        possible_return_types_state,
        metadata,
        ..
    } = property
    else {
        return SemanticResolution::Unresolved {
            reason: "property access could not be resolved, so count-versus-property semantics are unresolved".to_owned(),
            metadata: vec![metadata(
                "semantic-mode",
                &format!("{mode}-count-or-property"),
            )],
        };
    };
    let mut possible = possible_return_types;
    possible.push("java.lang.Long".to_owned());
    possible.sort();
    possible.dedup();
    let return_type = match catalog::common_assignable_class(&possible) {
        Ok(Some(return_type)) if return_type != "java.lang.Object" => return_type,
        Ok(Some(_)) | Ok(None) => {
            return SemanticResolution::Unresolved {
                reason: "count-versus-property results have no known concrete common type"
                    .to_owned(),
                metadata,
            };
        }
        Err(reason) => {
            return SemanticResolution::Unresolved {
                reason: format!("count-versus-property common return type is unresolved: {reason}"),
                metadata,
            };
        }
    };
    resolved_with_possible_types(
        return_type,
        possible,
        if possible_return_types_state == ExpressionPossibleReturnTypesState::Complete {
            ExpressionPossibleReturnTypesState::Partial
        } else {
            possible_return_types_state
        },
        DynamicMultiplicity::Both,
        metadata,
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
    // PropertyBaseExpression.getPropertyReturnTypes() deliberately excludes Object.class. A
    // handler may use Object only as a registration placeholder; it is not a possible value type.
    return_types.retain(|return_type| return_type != "java.lang.Object");
    let mut metadata = vec![metadata("semantic-mode", mode)];
    metadata.push(
        catalog::change_contract_metadata(
            subject_id,
            &catalog::property_change_contract(options, source),
        )
        .expect("an in-memory change contract must serialize"),
    );
    if return_types.is_empty() {
        return SemanticResolution::Unresolved {
            reason: "the matching property registration does not expose a concrete return type"
                .to_owned(),
            metadata,
        };
    }
    let return_type = match return_types.as_slice() {
        [only] => only.clone(),
        // PropertyBaseExpression.init() calls Utils.getSuperType(returnTypes). Ask the native
        // Catalog to perform the same hierarchy walk instead of collapsing every union to Object.
        _ => match catalog::common_assignable_class(&return_types) {
            Ok(Some(return_type)) => return_type,
            Ok(None) => {
                return SemanticResolution::Reject(
                    "matching property return types have no known common Java type".to_owned(),
                );
            }
            Err(reason) => {
                return SemanticResolution::Reject(format!(
                    "could not resolve the common property return type: {reason}"
                ));
            }
        },
    };
    resolved_with_possible_types(
        return_type,
        return_types,
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        metadata,
    )
}

pub(super) fn source_child(
    payload: &RegisteredExpressionPayload,
) -> Option<&RegisteredExpressionChild> {
    payload.children.iter().find(|child| {
        metadata_value(&child.metadata, "semantic-role") != Some("target-type")
            && metadata_value(&child.metadata, "target-class").is_none()
    })
}
