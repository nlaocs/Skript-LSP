use std::collections::BTreeMap;

use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::catalog::{self, AcceptedChangeType, ChangeContract, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprCmdCooldownInfo";
const HANDLER_ID: &str = "core.expression.expr-cmd-cooldown-info";
const SCRIPT_COMMAND_EVENT: &str = "ch.njol.skript.command.ScriptCommandEvent";
const TIMESPAN: &str = "ch.njol.skript.util.Timespan";
const DATE: &str = "ch.njol.skript.util.Date";
const STRING: &str = "java.lang.String";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_cooldown_info(payload))
}

fn resolve_cooldown_info(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some((return_type, mode)) = pattern_semantics(payload.pattern_index) else {
        return SemanticResolution::Reject(
            "command cooldown Expression has an unknown pattern index".to_owned(),
        );
    };
    let mut unknown_event_relation = false;
    let mut supported_event = false;
    for event_class in &payload.context.event_classes {
        match catalog::is_class_assignable(event_class, SCRIPT_COMMAND_EVENT) {
            Ok(TypeRelation::Compatible) => supported_event = true,
            Ok(TypeRelation::Incompatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => unknown_event_relation = true,
        }
    }
    if !supported_event {
        if unknown_event_relation {
            return SemanticResolution::Unresolved {
                reason: "whether the current event is a ScriptCommandEvent is unresolved"
                    .to_owned(),
                metadata: vec![metadata("semantic-mode", mode)],
            };
        }
        return SemanticResolution::Reject(
            "command cooldown Expression is only available in command events".to_owned(),
        );
    }

    let mut output_metadata = vec![metadata("semantic-mode", mode)];
    if let Some(contract) = change_contract_for_pattern(payload.pattern_index)
        && let Ok(contract) = catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        output_metadata.push(contract);
    }
    resolved_with_possible_types(
        return_type.to_owned(),
        vec![return_type.to_owned()],
        ExpressionPossibleReturnTypesState::Complete,
        DynamicMultiplicity::Single,
        output_metadata,
    )
}

fn pattern_semantics(pattern_index: u64) -> Option<(&'static str, &'static str)> {
    match pattern_index {
        0 => Some((TIMESPAN, "remaining-time")),
        1 => Some((TIMESPAN, "elapsed-time")),
        2 => Some((TIMESPAN, "cooldown-time")),
        3 => Some((DATE, "last-usage-date")),
        4 => Some((STRING, "bypass-permission")),
        _ => None,
    }
}

fn change_contract_for_pattern(pattern_index: u64) -> Option<ChangeContract> {
    let (class_name, modes) = match pattern_index {
        0 | 1 => (TIMESPAN, ["ADD", "REMOVE", "RESET", "SET"].as_slice()),
        3 => (DATE, ["REMOVE_ALL", "RESET", "SET"].as_slice()),
        _ => return None,
    };
    Some(ChangeContract::Resolved {
        modes: modes
            .iter()
            .map(|mode| {
                (
                    (*mode).to_owned(),
                    vec![AcceptedChangeType {
                        class_name: class_name.to_owned(),
                        multiple: false,
                    }],
                )
            })
            .collect::<BTreeMap<_, _>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::{DATE, STRING, TIMESPAN, change_contract_for_pattern, pattern_semantics};
    use crate::catalog::ChangeContract;

    #[test]
    fn maps_the_five_native_patterns_to_their_return_types() {
        assert_eq!(pattern_semantics(0), Some((TIMESPAN, "remaining-time")));
        assert_eq!(pattern_semantics(3), Some((DATE, "last-usage-date")));
        assert_eq!(pattern_semantics(4), Some((STRING, "bypass-permission")));
        assert_eq!(pattern_semantics(5), None);
    }

    #[test]
    fn only_mutable_cooldown_views_publish_change_contracts() {
        let ChangeContract::Resolved { modes } = change_contract_for_pattern(0).unwrap() else {
            panic!("remaining time must be changeable");
        };
        assert!(modes.contains_key("ADD"));
        assert!(modes.contains_key("SET"));
        assert!(change_contract_for_pattern(2).is_none());
        assert!(change_contract_for_pattern(4).is_none());
    }
}
