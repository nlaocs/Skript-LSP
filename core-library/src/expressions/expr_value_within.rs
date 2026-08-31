use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, MetadataEntry, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprValueWithin";
const HANDLER_ID: &str = "core.expression.expr-value-within";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_value_within(payload))
}

fn resolve_value_within(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let (target_type, requested_plural, source) = match target_and_source(payload) {
        Ok(value) => value,
        Err(TargetSourceError::Reject(reason)) => return SemanticResolution::Reject(reason),
        Err(TargetSourceError::Unresolved(reason)) => {
            return SemanticResolution::Unresolved {
                reason,
                metadata: vec![metadata("semantic-mode", "value-within")],
            };
        }
    };

    let Some(source_multiplicity) = source.multiplicity else {
        return SemanticResolution::Unresolved {
            reason: "value within cannot determine the source multiplicity".to_owned(),
            metadata: vec![metadata("semantic-mode", "value-within")],
        };
    };
    if source_multiplicity == DynamicMultiplicity::Both {
        return SemanticResolution::Unresolved {
            reason: "value within cannot determine whether the source is singular or plural"
                .to_owned(),
            metadata: vec![metadata("semantic-mode", "value-within")],
        };
    }

    let source_is_plural = source_multiplicity == DynamicMultiplicity::Multiple;
    if requested_plural != source_is_plural {
        return if requested_plural {
            SemanticResolution::Reject(
                "value within cannot get multiple elements of a single value".to_owned(),
            )
        } else {
            SemanticResolution::Reject(
                "the source may contain more than one value; use the plural form".to_owned(),
            )
        };
    }

    let return_type = if let Some(target_type) = target_type {
        match conversion_status(source, &target_type) {
            ConversionStatus::Compatible => target_type,
            ConversionStatus::Incompatible => {
                return SemanticResolution::Reject(format!(
                    "the source cannot be converted to {target_type}"
                ));
            }
            ConversionStatus::Unknown => {
                return SemanticResolution::Unresolved {
                    reason: format!("conversion of the source to {target_type} is unresolved"),
                    metadata: vec![metadata("semantic-mode", "value-within")],
                };
            }
        }
    } else {
        let Some(return_type) = source.return_type.clone() else {
            return SemanticResolution::Unresolved {
                reason: "value within has no source return type information".to_owned(),
                metadata: vec![metadata("semantic-mode", "value-within")],
            };
        };
        return_type
    };

    let typed = target_type_is_present(payload);
    let possible_return_types = if typed || source.possible_return_types.is_empty() {
        vec![return_type.clone()]
    } else {
        source.possible_return_types.clone()
    };
    let mut output_metadata = vec![metadata("semantic-mode", "value-within")];
    append_source_capabilities(&mut output_metadata, source);
    if let Ok(Some(contract)) = crate::catalog::type_change_contract(&return_type)
        && let Ok(contract) =
            crate::catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        output_metadata.push(contract);
    }
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        if typed {
            crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete
        } else {
            source.possible_return_types_state
        },
        source_multiplicity,
        output_metadata,
    )
}

enum TargetSourceError {
    Reject(String),
    Unresolved(String),
}

fn target_and_source(
    payload: &RegisteredExpressionPayload,
) -> Result<(Option<String>, bool, &RegisteredExpressionChild), TargetSourceError> {
    let target = payload
        .children
        .iter()
        .find(|child| metadata_value(&child.metadata, "target-class").is_some());
    let (target_type, requested_plural) = if let Some(target) = target {
        let target_type = metadata_value(&target.metadata, "target-class")
            .expect("target-class was checked above")
            .to_owned();
        let Some(plural) = metadata_value(&target.metadata, "type-plural") else {
            return Err(TargetSourceError::Unresolved(
                "typed value within has no ClassInfo plurality".to_owned(),
            ));
        };
        let Some(plural) = parse_bool(plural) else {
            return Err(TargetSourceError::Unresolved(
                "typed value within has an invalid ClassInfo plurality".to_owned(),
            ));
        };
        (Some(target_type), plural)
    } else {
        (None, payload.tags.iter().any(|tag| tag.value == "s"))
    };

    let source = if target.is_some() {
        payload
            .children
            .iter()
            .find(|child| metadata_value(&child.metadata, "target-class").is_none())
    } else {
        payload.children.first()
    }
    .ok_or_else(|| {
        TargetSourceError::Reject("value within requires a source Expression".to_owned())
    })?;
    Ok((target_type, requested_plural, source))
}

fn conversion_status(source: &RegisteredExpressionChild, target_type: &str) -> ConversionStatus {
    let source_types = if source.possible_return_types.is_empty() {
        source.return_type.iter().cloned().collect::<Vec<_>>()
    } else {
        source.possible_return_types.clone()
    };
    if source_types.is_empty() {
        return ConversionStatus::Unknown;
    }

    let mut unknown = false;
    let mut compatible = false;
    for source_type in source_types {
        if source_type == target_type {
            compatible = true;
            continue;
        }
        match crate::catalog::can_convert(&source_type, target_type) {
            Ok(TypeRelation::Compatible) => compatible = true,
            Ok(TypeRelation::Incompatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => unknown = true,
        }
    }
    if unknown {
        ConversionStatus::Unknown
    } else if compatible {
        ConversionStatus::Compatible
    } else {
        ConversionStatus::Incompatible
    }
}

fn target_type_is_present(payload: &RegisteredExpressionPayload) -> bool {
    payload
        .children
        .iter()
        .any(|child| metadata_value(&child.metadata, "target-class").is_some())
}

fn append_source_capabilities(output: &mut Vec<MetadataEntry>, source: &RegisteredExpressionChild) {
    for key in [
        "expression.capability.key-provider",
        "expression.capability.nested-structures",
    ] {
        if let Some(value) = metadata_value(&source.metadata, key) {
            output.push(metadata(key, value));
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConversionStatus {
    Compatible,
    Incompatible,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::{ConversionStatus, parse_bool};

    #[test]
    fn value_within_plurality_matches_the_wrapped_expression_shape() {
        assert!(plurality_matches(false, "single"));
        assert!(plurality_matches(true, "multiple"));
        assert!(!plurality_matches(false, "multiple"));
        assert!(!plurality_matches(true, "single"));
    }

    #[test]
    fn invalid_plurality_metadata_is_not_interpreted_as_a_boolean() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("unknown"), None);
    }

    #[test]
    fn conversion_has_a_separate_unknown_state() {
        assert_ne!(ConversionStatus::Compatible, ConversionStatus::Unknown);
        assert_ne!(ConversionStatus::Incompatible, ConversionStatus::Unknown);
    }

    fn plurality_matches(requested_plural: bool, source_shape: &str) -> bool {
        requested_plural == (source_shape == "multiple")
    }
}
