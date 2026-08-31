use crate::catalog::{self, ChangeContract, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, ParseResultStatus, ParseSummary, ParsedCapture,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffZombify";
const HANDLER_ID: &str = "core.effect.eff-zombify";
const LIVING_ENTITY: &str = "org.bukkit.entity.LivingEntity";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }

    let (pattern_index, pattern) = {
        let candidate = payload.candidate.as_ref()?;
        (candidate.pattern_index, candidate.pattern.clone())
    };
    let Some((mode, expected_pattern)) = pattern_info(pattern_index) else {
        return Some(unresolved(
            payload,
            "core.eff-zombify.unknown-pattern",
            "this EffZombify pattern is not known to CoreLibrary",
        ));
    };
    if pattern.trim() != expected_pattern {
        return Some(unresolved(
            payload,
            "core.eff-zombify.unknown-pattern",
            "this EffZombify pattern is not known to CoreLibrary",
        ));
    }
    super::annotate(&mut payload, "semantic-mode", mode);
    super::annotate(&mut payload, "native-pattern", expected_pattern);
    if has_optional_timespan(pattern_index) {
        let timespan_source = if successful_capture(&payload, 1).is_some() {
            "explicit"
        } else {
            "immediate"
        };
        super::annotate(&mut payload, "timespan-source", timespan_source);
    }

    let Some(entities) = successful_capture(&payload, 0) else {
        return Some(unresolved(
            payload,
            "core.eff-zombify.unresolved-entities",
            "the living entities Expression could not be inspected",
        ));
    };
    let Some(summary) = entities.summary.as_ref() else {
        return Some(unresolved(
            payload,
            "core.eff-zombify.unresolved-change-contract",
            "the living entities Expression has no semantic summary, so its SET changer could not be inspected",
        ));
    };

    let change_in_place = match accepts_set_living_entity(entities, summary) {
        Ok(Some(value)) => value,
        Ok(None) | Err(_) => {
            return Some(unresolved(
                payload,
                "core.eff-zombify.unresolved-change-contract",
                "the living entities Expression's SET changer could not be resolved",
            ));
        }
    };
    super::annotate(
        &mut payload,
        "change-in-place",
        if change_in_place { "true" } else { "false" },
    );
    Some(super::accept(payload))
}

fn successful_capture(
    payload: &EffectPayload,
    capture_index: u64,
) -> Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture> {
    super::parsed_capture(payload, capture_index)
        .filter(|capture| capture.status == ParseResultStatus::Success)
}

fn pattern_info(pattern_index: u64) -> Option<(&'static str, &'static str)> {
    match pattern_index {
        0 => Some(("zombify", "zombify %livingentities%")),
        1 => Some((
            "unzombify",
            "unzombify %livingentities% [(in|after) %-timespan%]",
        )),
        _ => None,
    }
}

const fn has_optional_timespan(pattern_index: u64) -> bool {
    pattern_index == 1
}

fn accepts_set_living_entity(
    capture: &ParsedCapture,
    summary: &ParseSummary,
) -> Result<Option<bool>, String> {
    // Variables are handled by Skript's generic Variable Expression changer and can be
    // updated in place regardless of the value's static class.
    if summary.kind == "variable" {
        return Ok(Some(true));
    }

    let subject_id = summary
        .registration_id
        .as_deref()
        .unwrap_or(capture.parser_id.as_str());
    let contract = match catalog::change_contract_from_metadata(&summary.metadata, subject_id)? {
        Some(contract) => Some(contract),
        None => super::eff_change::source_change_contract(summary)?,
    };
    let Some(ChangeContract::Resolved { modes }) = contract else {
        return Ok(None);
    };
    let Some(accepted_types) = modes.get("SET") else {
        return Ok(Some(false));
    };

    let mut unknown = false;
    for accepted in accepted_types {
        // This intentionally uses Java's assignability relation, not converters. It mirrors
        // ChangerUtils.acceptsChange(validTypes, LivingEntity.class).
        match catalog::is_class_assignable(LIVING_ENTITY, &accepted.class_name)? {
            TypeRelation::Compatible => return Ok(Some(true)),
            TypeRelation::Incompatible => {}
            TypeRelation::Unknown => unknown = true,
        }
    }
    Ok(if unknown { None } else { Some(false) })
}

fn unresolved(mut payload: EffectPayload, code: &str, message: &str) -> HookOutput {
    let span = payload
        .candidate
        .as_ref()
        .map(|candidate| candidate.span.clone())
        .unwrap_or_else(|| payload.span.clone());
    super::mark_unresolved(&mut payload, code);
    super::continue_with_diagnostics(payload, vec![super::warning(code, message, span)])
}

#[cfg(test)]
mod tests {
    #[test]
    fn pattern_zero_is_zombify_without_a_timespan() {
        assert_eq!(super::pattern_info(0).map(|info| info.0), Some("zombify"));
        assert!(!super::has_optional_timespan(0));
    }

    #[test]
    fn pattern_one_is_unzombify_with_an_optional_timespan() {
        assert_eq!(super::pattern_info(1).map(|info| info.0), Some("unzombify"));
        assert!(super::has_optional_timespan(1));
    }
}
