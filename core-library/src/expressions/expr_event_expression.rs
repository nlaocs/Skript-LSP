use super::{SemanticResolution, matches, register_handler_with_all_type_options};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprEventExpression";
const HANDLER_ID: &str = "core.expression.expr-event-expression";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_all_type_options(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_event_expression(payload))
}

fn resolve_event_expression(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(input) = payload.regex_captures.first().map(|value| value.trim()) else {
        return SemanticResolution::Reject(
            "event Expression has no type or event-value identifier".to_owned(),
        );
    };
    if input.is_empty() {
        return SemanticResolution::Reject(
            "event Expression has an empty type or event-value identifier".to_owned(),
        );
    }

    let Some((type_option, plural)) =
        crate::types::match_user_type_option(input, &payload.type_options)
    else {
        return super::event_value_expression::resolve_identifier(payload, input);
    };

    let (return_type, multiplicity) = event_value_target(&type_option.class_name, plural);
    add_input_metadata(
        // EventValueExpression keeps the component class as getReturnType(); its array class is
        // an internal representation used to select the plural EventValue registration.
        super::event_value_expression::resolve_target(payload, &return_type, Some(multiplicity)),
        input,
        &type_option.code_name,
    )
}

fn add_input_metadata(
    resolution: SemanticResolution,
    input: &str,
    code_name: &str,
) -> SemanticResolution {
    match resolution {
        SemanticResolution::Resolved {
            return_type,
            possible_return_types,
            possible_return_types_state,
            multiplicity,
            metadata: mut entries,
        } => {
            super::set_metadata(&mut entries, "semantic-mode", "event-expression");
            entries.push(super::metadata("event-expression-type", code_name));
            entries.push(super::metadata("event-expression-input", input));
            SemanticResolution::Resolved {
                return_type,
                possible_return_types,
                possible_return_types_state,
                multiplicity,
                metadata: entries,
            }
        }
        SemanticResolution::Unresolved {
            reason,
            metadata: mut entries,
        } => {
            super::set_metadata(&mut entries, "semantic-mode", "event-expression");
            entries.push(super::metadata("event-expression-type", code_name));
            entries.push(super::metadata("event-expression-input", input));
            SemanticResolution::Unresolved {
                reason,
                metadata: entries,
            }
        }
        SemanticResolution::Reject(reason) => SemanticResolution::Reject(reason),
    }
}

fn event_value_target(class_name: &str, plural: bool) -> (String, DynamicMultiplicity) {
    (
        class_name.to_owned(),
        if plural {
            DynamicMultiplicity::Multiple
        } else {
            DynamicMultiplicity::Single
        },
    )
}

#[cfg(test)]
mod tests {
    use super::event_value_target;
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    #[test]
    fn event_type_plurality_changes_multiplicity_but_not_return_type() {
        assert_eq!(
            event_value_target("org.example.Player", false),
            ("org.example.Player".to_owned(), DynamicMultiplicity::Single)
        );
        assert_eq!(
            event_value_target("org.example.Player", true),
            (
                "org.example.Player".to_owned(),
                DynamicMultiplicity::Multiple
            )
        );
    }
}
