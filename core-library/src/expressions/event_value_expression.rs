use super::{SemanticResolution, matches, metadata, resolved_with_possible_types};
use crate::catalog::{ChangeContract, EventValueOption, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use parser_wasm::REGISTERED_CONTEXT_ALL_TYPE_OPTIONS;

const HANDLER_ID: &str = "core.expression.event-value";
const SUPER_CLASS: &str = "ch.njol.skript.expressions.base.EventValueExpression";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Expression,
        targets: vec![RegisteredSyntaxHandlerTarget::SuperClass(
            SUPER_CLASS.to_owned(),
        )],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: vec![REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()],
    });
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if !matches(payload, HANDLER_ID) || !is_skript_definition(&payload.definition_id) {
        return None;
    }
    let return_type = payload.declared_return_type.as_deref()?;
    Some(resolve_target(
        payload,
        return_type,
        payload.declared_multiplicity,
    ))
}

fn is_skript_definition(definition_id: &str) -> bool {
    definition_id.starts_with("expression:skript:")
}

pub(super) fn resolve_target(
    payload: &RegisteredExpressionPayload,
    return_type: &str,
    declared_multiplicity: Option<DynamicMultiplicity>,
) -> SemanticResolution {
    if payload.context.event_classes.is_empty() {
        return SemanticResolution::Reject(format!(
            "there is no {return_type} event value outside an event"
        ));
    }

    let targets = target_types(return_type, declared_multiplicity);
    let mut resolved = Vec::new();
    let mut ambiguous = false;
    for (target_type, multiplicity) in &targets {
        let mut target_matches = Vec::new();
        let mut target_unknown = false;
        let mut duplicate_event = None;
        for event_class in &payload.context.event_classes {
            let values = match crate::catalog::event_values_for(event_class) {
                Ok(values) => values,
                Err(reason) => {
                    return SemanticResolution::Unresolved {
                        reason: format!("event value catalog lookup failed: {reason}"),
                        metadata: vec![metadata("semantic-mode", "event-value")],
                    };
                }
            };
            let without_conversion = preferred_event_value_matches(
                &values,
                event_class,
                target_type,
                payload.time,
                false,
                &payload.type_options,
            );
            if let Some(message) = without_conversion.abort.as_ref() {
                return SemanticResolution::Reject(message.clone());
            }
            target_unknown |= without_conversion.unknown;
            if without_conversion.values.len() > 1 {
                duplicate_event = Some(event_class.as_str());
                break;
            }
            let matches = if without_conversion.values.is_empty() {
                let converted = preferred_event_value_matches(
                    &values,
                    event_class,
                    target_type,
                    payload.time,
                    true,
                    &payload.type_options,
                );
                if let Some(message) = converted.abort.as_ref() {
                    return SemanticResolution::Reject(message.clone());
                }
                converted
            } else {
                without_conversion
            };
            target_unknown |= matches.unknown;
            target_matches.extend(matches.values);
        }
        if let Some(event_class) = duplicate_event {
            if targets.len() == 1 {
                return SemanticResolution::Reject(format!(
                    "there are multiple {return_type} event values in {event_class}"
                ));
            }
            ambiguous = true;
        } else if target_matches.is_empty() {
            ambiguous |= target_unknown;
        } else {
            resolved.push((*multiplicity, target_matches));
        }
    }
    if resolved.len() > 1 || (targets.len() > 1 && ambiguous) {
        return SemanticResolution::Unresolved {
            reason: format!("the multiplicity of the {return_type} event value is unresolved"),
            metadata: vec![metadata("semantic-mode", "event-value")],
        };
    }
    let Some((multiplicity, matches)) = resolved.pop() else {
        return if ambiguous {
            SemanticResolution::Unresolved {
                reason: format!("the {return_type} event value could not be resolved"),
                metadata: vec![metadata("semantic-mode", "event-value")],
            }
        } else {
            SemanticResolution::Reject(format!(
                "there is no {return_type} event value in the current event"
            ))
        };
    };

    let registration_ids = matches
        .iter()
        .map(|value| value.registration_id.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut metadata_entries = vec![
        metadata("semantic-mode", "event-value"),
        metadata("event-value-registration-ids", &registration_ids),
    ];
    if matches
        .iter()
        .any(|value| value.context_dependent == Some(true))
    {
        metadata_entries.push(metadata("context-dependent", "true"));
        metadata_entries.push(metadata("revalidate-on-context-change", "true"));
    }
    if let Some(modes) = event_value_changer_modes(&matches) {
        metadata_entries.push(metadata("event-value-changer-modes", &modes));
    }
    if let Some(contract) = event_value_change_contract(return_type, &matches)
        && let Ok(contract) =
            crate::catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        metadata_entries.push(contract);
    }
    resolved_with_possible_types(
        return_type.to_owned(),
        vec![return_type.to_owned()],
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        metadata_entries,
    )
}

pub(super) fn resolve_identifier(
    payload: &RegisteredExpressionPayload,
    identifier: &str,
) -> SemanticResolution {
    if payload.context.event_classes.is_empty() {
        return SemanticResolution::Reject(format!(
            "there is no event-{identifier} outside an event"
        ));
    }

    let mut selected = Vec::new();
    let mut unknown = false;
    for event_class in &payload.context.event_classes {
        let values = match crate::catalog::event_values_for_input(event_class, identifier) {
            Ok(values) => values,
            Err(reason) => {
                return SemanticResolution::Unresolved {
                    reason: format!("event value identifier lookup failed: {reason}"),
                    metadata: vec![metadata("semantic-mode", "event-value-identifier")],
                };
            }
        };
        let matches = preferred_identifier_matches(&values, event_class, payload.time);
        if let Some(message) = matches.abort.as_ref() {
            return SemanticResolution::Reject(message.clone());
        }
        unknown |= matches.unknown;
        if matches.values.len() > 1 {
            return SemanticResolution::Reject(format!(
                "there are multiple event values matching {identifier:?} in {event_class}"
            ));
        }
        selected.extend(matches.values);
    }

    if selected.is_empty() {
        return if unknown {
            SemanticResolution::Unresolved {
                reason: format!("event value identifier {identifier:?} is unresolved"),
                metadata: vec![metadata("semantic-mode", "event-value-identifier")],
            }
        } else {
            SemanticResolution::Reject(format!(
                "there is no event value matching {identifier:?} in the current event"
            ))
        };
    }

    let mut possible_return_types = selected
        .iter()
        .map(|value| {
            value
                .value_class
                .strip_suffix("[]")
                .unwrap_or(&value.value_class)
                .to_owned()
        })
        .collect::<Vec<_>>();
    possible_return_types.sort();
    possible_return_types.dedup();
    let return_type = if possible_return_types.len() == 1 {
        possible_return_types[0].clone()
    } else {
        crate::catalog::common_assignable_class(&possible_return_types)
            .ok()
            .flatten()
            .unwrap_or_else(|| "java.lang.Object".to_owned())
    };
    let multiplicity = if selected
        .iter()
        .any(|value| value.value_class.ends_with("[]"))
    {
        DynamicMultiplicity::Multiple
    } else {
        DynamicMultiplicity::Single
    };
    let registration_ids = selected
        .iter()
        .map(|value| value.registration_id.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut output_metadata = vec![
        metadata("semantic-mode", "event-value-identifier"),
        metadata("event-value-identifier", identifier),
        metadata("event-value-registration-ids", &registration_ids),
    ];
    if selected
        .iter()
        .any(|value| value.context_dependent == Some(true))
    {
        output_metadata.push(metadata("context-dependent", "true"));
        output_metadata.push(metadata("revalidate-on-context-change", "true"));
    }
    if let Some(modes) = event_value_changer_modes(&selected) {
        output_metadata.push(metadata("event-value-changer-modes", &modes));
    }
    if let Some(contract) = event_value_change_contract(&return_type, &selected)
        && let Ok(contract) =
            crate::catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        output_metadata.push(contract);
    }
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        if unknown {
            ExpressionPossibleReturnTypesState::Partial
        } else {
            ExpressionPossibleReturnTypesState::Complete
        },
        multiplicity,
        output_metadata,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionPhase {
    Exact,
    Nearest,
    Downcast,
    Conversion,
}

struct CandidateMatches {
    values: Vec<EventValueOption>,
    unknown: bool,
    abort: Option<String>,
}

fn preferred_identifier_matches(
    values: &[EventValueOption],
    event_class: &str,
    requested_time: i32,
) -> CandidateMatches {
    let requested = collect_identifier_matches(values, event_class, requested_time);
    if requested_time == 0 || !requested.values.is_empty() || requested.abort.is_some() {
        return requested;
    }
    let fallback = collect_identifier_matches(values, event_class, 0);
    CandidateMatches {
        values: fallback.values,
        unknown: requested.unknown || fallback.unknown,
        abort: fallback.abort,
    }
}

fn collect_identifier_matches(
    values: &[EventValueOption],
    event_class: &str,
    time: i32,
) -> CandidateMatches {
    let mut best_rank = None;
    let mut selected = Vec::new();
    let mut unknown = false;
    for value in values.iter().filter(|value| value.time == time) {
        let rank = match event_distance_rank(&value.event_class, event_class) {
            Ok(Some(rank)) => rank,
            Ok(None) => continue,
            Err(()) => {
                unknown = true;
                continue;
            }
        };
        match validate_candidate(value, event_class, true) {
            CandidateValidation::Abort(message) => {
                return CandidateMatches {
                    values: Vec::new(),
                    unknown,
                    abort: Some(message),
                };
            }
            CandidateValidation::Unknown => {
                unknown = true;
                continue;
            }
            CandidateValidation::Valid => {}
        }
        match best_rank {
            None => {
                best_rank = Some(rank);
                selected.push(value.clone());
            }
            Some(best) if rank < best => {
                best_rank = Some(rank);
                selected.clear();
                selected.push(value.clone());
            }
            Some(best) if rank == best => selected.push(value.clone()),
            Some(_) => {}
        }
    }
    CandidateMatches {
        values: selected,
        unknown,
        abort: None,
    }
}

/// Resolve one EventValue time state before falling back to the default state.
///
/// Skript gives a requested past/future state first chance. The default state is
/// consulted only when that state produced no compatible value for this target.
/// This matters when an event has an unrelated registration at the requested
/// time but a useful default registration for the expression being resolved.
fn preferred_event_value_matches(
    values: &[EventValueOption],
    event_class: &str,
    target_type: &str,
    requested_time: i32,
    allow_conversion: bool,
    type_options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> CandidateMatches {
    let requested = collect_event_value_matches(
        values,
        event_class,
        target_type,
        requested_time,
        allow_conversion,
        type_options,
    );
    if requested_time == 0 || !requested.values.is_empty() {
        return requested;
    }
    if requested.abort.is_some() {
        return requested;
    }

    let fallback = collect_event_value_matches(
        values,
        event_class,
        target_type,
        0,
        allow_conversion,
        type_options,
    );
    CandidateMatches {
        values: fallback.values,
        unknown: requested.unknown || fallback.unknown,
        abort: fallback.abort,
    }
}

fn collect_event_value_matches(
    values: &[EventValueOption],
    event_class: &str,
    target_type: &str,
    time: i32,
    allow_conversion: bool,
    type_options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> CandidateMatches {
    let phases = if allow_conversion {
        &[
            ResolutionPhase::Exact,
            ResolutionPhase::Nearest,
            ResolutionPhase::Downcast,
            ResolutionPhase::Conversion,
        ][..]
    } else {
        &[ResolutionPhase::Exact, ResolutionPhase::Nearest][..]
    };
    let mut unknown = false;
    for phase in phases {
        let mut matches = Vec::new();
        let mut best_rank = None;
        for value in values.iter().filter(|value| value.time == time) {
            match candidate_rank(value, event_class, target_type, *phase) {
                Ok(Some(rank)) => match validate_candidate(value, event_class, false) {
                    CandidateValidation::Abort(message) => {
                        return CandidateMatches {
                            values: Vec::new(),
                            unknown,
                            abort: Some(message),
                        };
                    }
                    CandidateValidation::Unknown => {
                        unknown = true;
                    }
                    CandidateValidation::Valid => match best_rank {
                        None => {
                            best_rank = Some(rank);
                            matches.push(value.clone());
                        }
                        Some(best) if rank < best => {
                            best_rank = Some(rank);
                            matches.clear();
                            matches.push(value.clone());
                        }
                        Some(best) if rank == best => matches.push(value.clone()),
                        Some(_) => {}
                    },
                },
                Ok(None) => {}
                Err(()) => unknown = true,
            }
        }
        if !matches.is_empty() {
            return CandidateMatches {
                values: strip_event_value_candidates(target_type, matches, type_options),
                unknown,
                abort: None,
            };
        }
    }
    CandidateMatches {
        values: Vec::new(),
        unknown,
        abort: None,
    }
}

enum CandidateValidation {
    Valid,
    Abort(String),
    Unknown,
}

fn validate_candidate(
    value: &EventValueOption,
    event_class: &str,
    validate_input: bool,
) -> CandidateValidation {
    for excluded in &value.excludes {
        match crate::catalog::is_class_assignable(event_class, excluded) {
            Ok(TypeRelation::Compatible) => {
                return CandidateValidation::Abort(
                    value.exclude_error_message.clone().unwrap_or_else(|| {
                        format!(
                            "event value {} is excluded from {event_class}",
                            value.registration_id
                        )
                    }),
                );
            }
            Ok(TypeRelation::Incompatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => return CandidateValidation::Unknown,
        }
    }
    if value.has_custom_event_validator == Some(true)
        || (validate_input && value.has_custom_input_validator == Some(true))
    {
        return CandidateValidation::Unknown;
    }
    CandidateValidation::Valid
}

fn candidate_rank(
    value: &EventValueOption,
    event_class: &str,
    target_type: &str,
    phase: ResolutionPhase,
) -> Result<Option<(i64, i64, i64)>, ()> {
    let target_is_multiple = target_type.ends_with("[]");
    if value.value_class.ends_with("[]") != target_is_multiple {
        return Ok(None);
    }
    let target_component = target_type.strip_suffix("[]").unwrap_or(target_type);
    let value_component = value
        .value_class
        .strip_suffix("[]")
        .unwrap_or(&value.value_class);
    let event_distances = event_distance_ranks(&value.event_class, event_class)?;
    let value_distance = match phase {
        ResolutionPhase::Exact => {
            let forward = crate::catalog::hierarchy_distance(&value.event_class, event_class)
                .map_err(|_| ())?;
            if forward.is_none() || value_component != target_component {
                return Ok(None);
            }
            0
        }
        ResolutionPhase::Nearest => {
            let Some(distance) =
                crate::catalog::hierarchy_distance(target_component, value_component)
                    .map_err(|_| ())?
            else {
                return Ok(None);
            };
            distance as i64
        }
        ResolutionPhase::Downcast => {
            if crate::catalog::hierarchy_distance(value_component, target_component)
                .map_err(|_| ())?
                .is_none()
            {
                return Ok(None);
            }
            // Resolver.EVENT_VALUE_DISTANCE_COMPARATOR calls hierarchyDistance in
            // this direction, which is -1 for every downcast candidate.
            -1
        }
        ResolutionPhase::Conversion => {
            match crate::catalog::can_convert(value_component, target_component).map_err(|_| ())? {
                TypeRelation::Compatible => 0,
                TypeRelation::Incompatible => return Ok(None),
                TypeRelation::Unknown => return Err(()),
            }
        }
    };
    let Some((forward, reverse)) = event_distances else {
        return Ok(None);
    };
    Ok(Some((forward, reverse, value_distance)))
}

fn event_distance_rank(registered: &str, current: &str) -> Result<Option<i64>, ()> {
    event_distance_ranks(registered, current).map(|ranks| ranks.map(|(forward, _)| forward))
}

fn event_distance_ranks(registered: &str, current: &str) -> Result<Option<(i64, i64)>, ()> {
    let forward = crate::catalog::hierarchy_distance(registered, current).map_err(|_| ())?;
    let reverse = crate::catalog::hierarchy_distance(current, registered).map_err(|_| ())?;
    if forward.is_none() && reverse.is_none() {
        return Ok(None);
    }
    // Skript's comparator uses ClassUtils.hierarchyDistance directly. Its -1
    // result sorts before non-negative distances, including for related classes
    // in the reverse direction.
    Ok(Some((
        forward.map_or(-1, |distance| distance as i64),
        reverse.map_or(-1, |distance| distance as i64),
    )))
}

/// Mirrors Resolver's `stripConverters` rule with the Catalog's type lookup.
///
/// A value that already has its own ClassInfo is not also used as a generic
/// converted value. Values without a ClassInfo remain eligible. If every
/// candidate would be removed, the original list is retained, matching Skript's
/// conservative fallback for an empty stripped result.
fn strip_event_value_candidates(
    requested_type: &str,
    candidates: Vec<EventValueOption>,
    type_options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> Vec<EventValueOption> {
    strip_event_value_candidates_with_class_info(requested_type, candidates, |class_name| {
        type_options
            .iter()
            .find(|option| option.class_name == class_name)
            .map(|option| option.registration_id.clone())
    })
}

fn strip_event_value_candidates_with_class_info<F>(
    requested_type: &str,
    candidates: Vec<EventValueOption>,
    class_info_of: F,
) -> Vec<EventValueOption>
where
    F: Fn(&str) -> Option<String> + Copy,
{
    if candidates.len() <= 1 {
        return candidates;
    }
    let requested_class_info = class_info_of(requested_type);
    let stripped = candidates
        .iter()
        .filter(|value| {
            class_info_of(&value.value_class)
                .is_none_or(|class_info| Some(class_info) == requested_class_info)
        })
        .cloned()
        .collect::<Vec<_>>();
    if stripped.is_empty() {
        candidates
    } else {
        stripped
    }
}

fn target_types(
    return_type: &str,
    declared: Option<DynamicMultiplicity>,
) -> Vec<(String, DynamicMultiplicity)> {
    let component = return_type.strip_suffix("[]").unwrap_or(return_type);
    match declared {
        Some(DynamicMultiplicity::Single) => {
            vec![(component.to_owned(), DynamicMultiplicity::Single)]
        }
        Some(DynamicMultiplicity::Multiple) => {
            vec![(format!("{component}[]"), DynamicMultiplicity::Multiple)]
        }
        Some(DynamicMultiplicity::Both) | None => vec![
            (component.to_owned(), DynamicMultiplicity::Single),
            (format!("{component}[]"), DynamicMultiplicity::Multiple),
        ],
    }
}

fn event_value_change_contract(
    return_type: &str,
    matches: &[EventValueOption],
) -> Option<ChangeContract> {
    let ChangeContract::Resolved { mut modes } =
        crate::catalog::type_change_contract(return_type).ok()??
    else {
        return Some(ChangeContract::Unresolved);
    };
    let event_modes = matches
        .iter()
        .flat_map(|value| value.accepted_changers.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    for mode in event_modes {
        if let Some(accepted) = matches
            .iter()
            .find_map(|value| value.accepted_changers.get(&mode))
        {
            modes.insert(mode, accepted.clone());
        }
    }
    Some(ChangeContract::Resolved { modes })
}

fn event_value_changer_modes(matches: &[EventValueOption]) -> Option<String> {
    let modes = matches
        .iter()
        .flat_map(|value| value.accepted_changers.keys().map(String::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    (!modes.is_empty()).then(|| modes.into_iter().collect::<Vec<_>>().join(";"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        EventValueOption, collect_identifier_matches, is_skript_definition,
        preferred_event_value_matches, strip_event_value_candidates_with_class_info, target_types,
    };
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    fn event_value(value_class: &str, time: i32, registration_id: &str) -> EventValueOption {
        EventValueOption {
            event_class: "test.Event".to_owned(),
            value_class: value_class.to_owned(),
            time,
            registration_id: registration_id.to_owned(),
            patterns: Vec::new(),
            excludes: Vec::new(),
            exclude_error_message: None,
            resolution_order: 0,
            registration_order: None,
            accepted_changers: BTreeMap::new(),
            context_dependent: None,
            has_custom_input_validator: None,
            has_custom_event_validator: None,
        }
    }

    #[test]
    fn unresolved_event_value_multiplicity_checks_scalar_and_array_targets() {
        assert_eq!(
            target_types("java.lang.String", None),
            vec![
                ("java.lang.String".to_owned(), DynamicMultiplicity::Single),
                (
                    "java.lang.String[]".to_owned(),
                    DynamicMultiplicity::Multiple
                )
            ]
        );
    }

    #[test]
    fn core_handler_does_not_claim_addon_event_value_subclasses() {
        assert!(is_skript_definition("expression:skript:abc"));
        assert!(!is_skript_definition("expression:skript-reflect:abc"));
        assert!(!is_skript_definition("expression:custom-addon:abc"));
    }

    #[test]
    fn custom_identifier_validator_keeps_a_pattern_match_unresolved() {
        let mut value = event_value("java.lang.String", 0, "custom-input");
        value.has_custom_input_validator = Some(true);
        let matches = collect_identifier_matches(&[value], "test.Event", 0);
        assert!(matches.values.is_empty());
        assert!(matches.unknown);
    }

    #[test]
    fn custom_event_validator_keeps_a_typed_match_unresolved() {
        let mut value = event_value("java.lang.String", 0, "custom-event");
        value.has_custom_event_validator = Some(true);
        let matches = preferred_event_value_matches(
            &[value],
            "test.Event",
            "java.lang.String",
            0,
            false,
            &[],
        );
        assert!(matches.values.is_empty());
        assert!(matches.unknown);
    }

    #[test]
    fn requested_event_value_time_wins_over_default_time() {
        let values = vec![
            event_value("java.lang.String", 0, "now"),
            event_value("java.lang.String", -1, "past"),
        ];
        let matches = preferred_event_value_matches(
            &values,
            "test.Event",
            "java.lang.String",
            -1,
            false,
            &[],
        );
        assert_eq!(
            matches
                .values
                .iter()
                .map(|value| value.registration_id.as_str())
                .collect::<Vec<_>>(),
            vec!["past"]
        );
    }

    #[test]
    fn missing_requested_event_value_time_falls_back_only_after_matching() {
        let values = vec![
            event_value("java.lang.String", 0, "now"),
            event_value("java.lang.Integer", 1, "unrelated-future"),
        ];
        let matches =
            preferred_event_value_matches(&values, "test.Event", "java.lang.String", 1, false, &[]);
        assert_eq!(
            matches
                .values
                .iter()
                .map(|value| value.registration_id.as_str())
                .collect::<Vec<_>>(),
            vec!["now"]
        );
    }

    #[test]
    fn strip_converters_drops_typed_values_when_their_classinfo_is_redundant() {
        let player = event_value("org.bukkit.entity.Player", 0, "player");
        let villager = event_value("org.bukkit.entity.AbstractVillager", 0, "villager");
        let candidates = vec![player, villager];
        let stripped = strip_event_value_candidates_with_class_info(
            "org.bukkit.entity.Entity",
            candidates,
            |class_name| match class_name {
                "org.bukkit.entity.Entity" => Some("entity".to_owned()),
                "org.bukkit.entity.Player" => Some("player".to_owned()),
                "org.bukkit.entity.AbstractVillager" => None,
                _ => None,
            },
        );
        assert_eq!(
            stripped
                .iter()
                .map(|value| value.registration_id.as_str())
                .collect::<Vec<_>>(),
            vec!["villager"]
        );
    }
}
