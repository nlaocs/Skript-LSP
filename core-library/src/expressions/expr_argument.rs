use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprArgument";
const HANDLER_ID: &str = "core.expression.expr-argument";
const STRING: &str = "java.lang.String";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_argument(payload))
}

fn resolve_argument(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let script_arguments = command_arguments(payload);
    let script_command = script_arguments.is_some();
    let arguments = script_arguments.unwrap_or_default();
    if script_command && arguments.is_empty() {
        return SemanticResolution::Reject("this command has no arguments".to_owned());
    }

    let selected = match payload.pattern_index {
        0 => script_command.then(|| arguments.last().cloned()).flatten(),
        1 | 2 => {
            let Some(ordinal) = payload
                .regex_captures
                .first()
                .and_then(|value| ordinal(value))
            else {
                return SemanticResolution::Reject(
                    "argument Expression has no valid ordinal".to_owned(),
                );
            };
            if script_command {
                let Some(argument) = ordinal
                    .checked_sub(1)
                    .and_then(|index| arguments.get(index))
                    .cloned()
                else {
                    return SemanticResolution::Reject(format!(
                        "this command has no argument {ordinal}"
                    ));
                };
                Some(argument)
            } else {
                None
            }
        }
        3 if payload.mark == 1 => {
            if script_command {
                return SemanticResolution::Reject(
                    "all arguments cannot be used inside script commands".to_owned(),
                );
            }
            return string_result(DynamicMultiplicity::Multiple, "command-event-arguments");
        }
        3 => {
            if script_command {
                if arguments.len() != 1 {
                    return SemanticResolution::Reject(
                        "this command has multiple arguments; use an argument number".to_owned(),
                    );
                }
                arguments.first().cloned()
            } else {
                None
            }
        }
        4 | 5 => {
            if !script_command {
                return SemanticResolution::Reject(
                    "typed arguments are unavailable in command events".to_owned(),
                );
            }
            let Some(target) = payload
                .children
                .iter()
                .find_map(|child| metadata_value(&child.metadata, "target-class"))
            else {
                return SemanticResolution::Reject(
                    "typed argument requires a ClassInfo literal".to_owned(),
                );
            };
            let requested = payload
                .regex_captures
                .first()
                .and_then(|value| ordinal(value));
            let mut compatible = Vec::new();
            for argument in arguments {
                match crate::catalog::is_class_assignable(&argument.class_name, target) {
                    Ok(TypeRelation::Compatible) => compatible.push(argument),
                    Ok(TypeRelation::Incompatible) => {}
                    Ok(TypeRelation::Unknown) => {
                        return unresolved_type_relation(format!(
                            "argument type compatibility with {target} is unresolved"
                        ));
                    }
                    Err(reason) => {
                        return unresolved_type_relation(format!(
                            "argument type compatibility with {target} is unavailable: {reason}"
                        ));
                    }
                }
            }
            if compatible.is_empty() {
                return SemanticResolution::Reject(format!(
                    "this command has no {target} argument"
                ));
            }
            if let Some(ordinal) = requested {
                let Some(argument) = ordinal
                    .checked_sub(1)
                    .and_then(|index| compatible.get(index))
                    .cloned()
                else {
                    return SemanticResolution::Reject(format!(
                        "this command has no {target} argument {ordinal}"
                    ));
                };
                Some(argument)
            } else {
                // Preserve Skript's current implementation: the third matching argument
                // makes an unnumbered typed reference ambiguous; one or two select the last.
                if compatible.len() > 2 {
                    return SemanticResolution::Reject(format!(
                        "this command has multiple {target} arguments"
                    ));
                }
                compatible.last().cloned()
            }
        }
        _ => {
            return SemanticResolution::Reject(format!(
                "unknown argument Expression pattern index: {}",
                payload.pattern_index
            ));
        }
    };

    match selected {
        Some(argument) => resolved_with_possible_types(
            argument.class_name.clone(),
            vec![argument.class_name],
            ExpressionPossibleReturnTypesState::Complete,
            if argument.single {
                DynamicMultiplicity::Single
            } else {
                DynamicMultiplicity::Multiple
            },
            vec![metadata("semantic-mode", "script-command-argument")],
        ),
        None => string_result(DynamicMultiplicity::Single, "command-event-argument"),
    }
}

fn unresolved_type_relation(reason: impl Into<String>) -> SemanticResolution {
    SemanticResolution::Unresolved {
        reason: reason.into(),
        metadata: vec![metadata("semantic-mode", "script-command-argument")],
    }
}

fn string_result(multiplicity: DynamicMultiplicity, mode: &str) -> SemanticResolution {
    resolved_with_possible_types(
        STRING.to_owned(),
        vec![STRING.to_owned()],
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        vec![metadata("semantic-mode", mode)],
    )
}

#[derive(Clone)]
struct CommandArgument {
    class_name: String,
    single: bool,
}

fn command_arguments(payload: &RegisteredExpressionPayload) -> Option<Vec<CommandArgument>> {
    let count = context_value(payload, "core.command.argument-count")?
        .parse()
        .ok()?;
    (0..count)
        .map(|index| {
            Some(CommandArgument {
                class_name: context_value(
                    payload,
                    &format!("core.command.argument.{index}.class"),
                )?
                .to_owned(),
                single: context_value(payload, &format!("core.command.argument.{index}.single"))?
                    == "true",
            })
        })
        .collect()
}

fn context_value<'a>(payload: &'a RegisteredExpressionPayload, key: &str) -> Option<&'a str> {
    payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

fn ordinal(value: &str) -> Option<usize> {
    let digits = value
        .trim()
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok().filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::{SemanticResolution, ordinal, unresolved_type_relation};

    #[test]
    fn extracts_both_plain_and_ordered_argument_numbers() {
        assert_eq!(ordinal("12"), Some(12));
        assert_eq!(ordinal("the 21st"), Some(21));
        assert_eq!(ordinal("argument"), None);
        assert_eq!(ordinal("0"), None);
    }

    #[test]
    fn unknown_type_relations_are_unresolved() {
        assert!(matches!(
            unresolved_type_relation("type relation is unavailable"),
            SemanticResolution::Unresolved { .. }
        ));
    }
}
