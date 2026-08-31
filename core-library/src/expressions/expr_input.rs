use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprInput";
const HANDLER_ID: &str = "core.expression.expr-input";
const INPUT_AVAILABLE: &str = "core.input-source.available";
const INPUT_HAS_INDICES: &str = "core.input-source.has-indices";
const INPUT_VALUE_TYPES: &str = "core.input-source.value-types";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_input(payload))
}

fn resolve_input(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    if context_value(payload, INPUT_AVAILABLE) != Some("true") {
        return SemanticResolution::Reject(
            "input Expression is only available inside an InputSource".to_owned(),
        );
    }

    let (return_type, possible_return_types, semantic_mode) = match payload.pattern_index {
        0 => {
            let source_types = context_value(payload, INPUT_VALUE_TYPES)
                .into_iter()
                .flat_map(|value| value.split(';'))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let possible = if source_types.is_empty() {
                vec!["java.lang.Object".to_owned()]
            } else {
                source_types
            };
            let effective = payload
                .expected_types
                .first()
                .map(|expected| expected.class_name.clone())
                .unwrap_or_else(|| common_or_object(&possible));
            (effective, possible, "input-value")
        }
        1 => {
            let Some(class_info) = payload.children.iter().find_map(|child| {
                let target = metadata_value(&child.metadata, "target-class")?;
                Some((
                    target,
                    metadata_value(&child.metadata, "type-plural") == Some("true"),
                ))
            }) else {
                return SemanticResolution::Reject(
                    "typed input Expression requires a resolved ClassInfo literal".to_owned(),
                );
            };
            if class_info.1 {
                return SemanticResolution::Reject(
                    "an input can only be a single value; use a singular type".to_owned(),
                );
            }
            (
                class_info.0.to_owned(),
                vec![class_info.0.to_owned()],
                "typed-input-value",
            )
        }
        2 => {
            if context_value(payload, INPUT_HAS_INDICES) != Some("true") {
                return SemanticResolution::Reject(
                    "input index is unavailable because this InputSource has no indices".to_owned(),
                );
            }
            (
                "java.lang.String".to_owned(),
                vec!["java.lang.String".to_owned()],
                "input-index",
            )
        }
        _ => {
            return SemanticResolution::Reject(format!(
                "unknown input Expression pattern index: {}",
                payload.pattern_index
            ));
        }
    };

    SemanticResolution::Resolved {
        return_type,
        possible_return_types,
        possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
        multiplicity: DynamicMultiplicity::Single,
        metadata: vec![metadata("semantic-mode", semantic_mode)],
    }
}

fn context_value<'a>(payload: &'a RegisteredExpressionPayload, key: &str) -> Option<&'a str> {
    payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

fn common_or_object(types: &[String]) -> String {
    match types {
        [only] => only.clone(),
        _ => "java.lang.Object".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::common_or_object;

    #[test]
    fn only_one_input_type_is_preserved() {
        assert_eq!(
            common_or_object(&["java.lang.String".to_owned()]),
            "java.lang.String"
        );
        assert_eq!(
            common_or_object(&["java.lang.String".to_owned(), "java.lang.Long".to_owned()]),
            "java.lang.Object"
        );
    }
}
