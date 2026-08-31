use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{EffectPayload, RegisteredSyntaxHandler};

const CLASS_SUFFIX: &str = ".EffToggle";
const HANDLER_ID: &str = "core.effect.eff-toggle";
const BLOCK: &str = "org.bukkit.block.Block";
const BOOLEAN: &str = "java.lang.Boolean";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate = payload.candidate.as_ref()?;
    let modern = crate::runtime::skript_at_least(2, 12)
        .unwrap_or_else(|| candidate.pattern.contains("booleans"));
    let Some(action) = toggle_action(modern, candidate.pattern_index) else {
        return Some(super::reject_with(
            "toggle Effect has an unknown pattern index",
            "core.eff-toggle.unknown-pattern",
            candidate.span.clone(),
        ));
    };
    super::annotate(&mut payload, "semantic-mode", "toggle-value");
    super::annotate(&mut payload, "toggle-action", action);
    if !modern || payload.candidate.as_ref()?.pattern_index != 2 {
        super::annotate(&mut payload, "toggle-value-kind", "block");
        return Some(super::accept(payload));
    }

    let Some(source) = super::parsed_capture(&payload, 0).cloned() else {
        let span = payload.candidate.as_ref()?.span.clone();
        return Some(unresolved(
            payload,
            "core.eff-toggle.missing-source",
            "the toggle target was not parsed",
            span,
        ));
    };
    let span = source.span.clone();
    let Some(summary) = source.summary.as_ref() else {
        return Some(unresolved(
            payload,
            "core.eff-toggle.unresolved-source",
            "the toggle target has no semantic summary",
            span,
        ));
    };
    let source_types = super::eff_change::source_types(summary);
    let block = can_return_any(&source_types, BLOCK);
    let boolean = can_return_any(&source_types, BOOLEAN);
    let Some(value_kind) = toggle_value_kind(block, boolean) else {
        return Some(unresolved(
            payload,
            "core.eff-toggle.unresolved-value-kind",
            "the toggle target's possible return types do not distinguish blocks from booleans",
            span,
        ));
    };
    super::annotate(&mut payload, "toggle-value-kind", value_kind.name());
    let required = match value_kind {
        ToggleValueKind::Blocks => return Some(super::accept(payload)),
        ToggleValueKind::Booleans => &[BOOLEAN][..],
        ToggleValueKind::Mixed => &[BLOCK, BOOLEAN][..],
    };
    match super::set_contract_verdict(&source, required) {
        Ok(super::ContractVerdict::Accepted) => Some(super::accept(payload)),
        Ok(super::ContractVerdict::Rejected) => Some(super::reject_with(
            format!(
                "the toggle target cannot be set to {}",
                required.join(" and ")
            ),
            "core.eff-toggle.unsupported-change",
            span,
        )),
        Ok(super::ContractVerdict::Unresolved) => Some(unresolved(
            payload,
            "core.eff-toggle.unresolved-change-contract",
            "the toggle target does not expose enough change data to validate the toggle",
            span,
        )),
        Err(reason) => Some(unresolved(
            payload,
            "core.eff-toggle.change-contract-unavailable",
            &format!("the toggle target's change contract could not be read: {reason}"),
            span,
        )),
    }
}

fn can_return_any(source_types: &[&str], target: &str) -> TypeRelation {
    let mut unknown = source_types.is_empty();
    for source in source_types {
        if *source == target {
            return TypeRelation::Compatible;
        }
        match crate::catalog::is_class_assignable(source, target) {
            Ok(TypeRelation::Compatible) => return TypeRelation::Compatible,
            Ok(TypeRelation::Incompatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => unknown = true,
        }
    }
    if unknown {
        TypeRelation::Unknown
    } else {
        TypeRelation::Incompatible
    }
}

fn unresolved(
    mut payload: EffectPayload,
    code: &str,
    message: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> crate::nlaocs::skript_parser_addon::types::HookOutput {
    super::mark_unresolved(&mut payload, code);
    super::continue_with_diagnostics(payload, vec![super::warning(code, message, span)])
}

fn toggle_action(modern: bool, pattern_index: u64) -> Option<&'static str> {
    match (modern, pattern_index) {
        (false, 0) | (true, 1) => Some("deactivate"),
        (false, 1) | (true, 2) => Some("toggle"),
        (false, 2) | (true, 0) => Some("activate"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToggleValueKind {
    Blocks,
    Booleans,
    Mixed,
}

impl ToggleValueKind {
    fn name(self) -> &'static str {
        match self {
            Self::Blocks => "block",
            Self::Booleans => "boolean",
            Self::Mixed => "mixed",
        }
    }
}

fn toggle_value_kind(blocks: TypeRelation, booleans: TypeRelation) -> Option<ToggleValueKind> {
    match (blocks, booleans) {
        (TypeRelation::Compatible, TypeRelation::Incompatible) => Some(ToggleValueKind::Blocks),
        (TypeRelation::Incompatible, TypeRelation::Compatible) => Some(ToggleValueKind::Booleans),
        (TypeRelation::Compatible, TypeRelation::Compatible)
        | (TypeRelation::Incompatible, TypeRelation::Incompatible) => Some(ToggleValueKind::Mixed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ToggleValueKind, toggle_action, toggle_value_kind};
    use crate::catalog::TypeRelation;

    #[test]
    fn keeps_legacy_and_boolean_capable_pattern_orders_distinct() {
        assert_eq!(toggle_action(false, 0), Some("deactivate"));
        assert_eq!(toggle_action(false, 2), Some("activate"));
        assert_eq!(toggle_action(true, 0), Some("activate"));
        assert_eq!(toggle_action(true, 2), Some("toggle"));
    }

    #[test]
    fn classifies_java_can_return_results() {
        assert_eq!(
            toggle_value_kind(TypeRelation::Incompatible, TypeRelation::Compatible),
            Some(ToggleValueKind::Booleans)
        );
        assert_eq!(
            toggle_value_kind(TypeRelation::Compatible, TypeRelation::Compatible),
            Some(ToggleValueKind::Mixed)
        );
        assert_eq!(
            toggle_value_kind(TypeRelation::Unknown, TypeRelation::Compatible),
            None
        );
    }
}
