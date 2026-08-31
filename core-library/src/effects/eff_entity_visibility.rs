use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, ParseResultStatus, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffEntityVisibility";
const HANDLER_ID: &str = "core.effect.eff-entity-visibility";
const HIDDEN_PLAYERS_CLASS: &str = "ch.njol.skript.expressions.ExprHiddenPlayers";
const HIDE_PATTERN: &str = "hide %entities% [(from|for) %-players%]";
const REVEAL_PATTERN: &str = "reveal %entities% [(to|for|from) %-players%]";

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
    let Some(mode) = pattern_info(pattern_index, &pattern).map(|info| info.0) else {
        return Some(unresolved(
            payload,
            "core.eff-entity-visibility.unknown-pattern",
            "this EffEntityVisibility pattern is not known to CoreLibrary",
        ));
    };
    super::annotate(&mut payload, "semantic-mode", mode);

    let Some(hidden) = successful_capture(&payload, 0) else {
        return Some(unresolved(
            payload,
            "core.eff-entity-visibility.unresolved-entities",
            "the entities Expression could not be inspected",
        ));
    };

    let Some(viewer_source) = viewer_source(
        pattern_index,
        hidden
            .summary
            .as_ref()
            .and_then(|summary| summary.element_class.as_deref()),
        successful_capture(&payload, 1).is_some(),
    ) else {
        return Some(unresolved(
            payload,
            "core.eff-entity-visibility.unresolved-viewer-source",
            "the reveal Expression's viewer source could not be resolved",
        ));
    };
    super::annotate(&mut payload, "viewer-source", viewer_source.as_str());

    Some(super::accept(payload))
}

fn successful_capture(
    payload: &EffectPayload,
    capture_index: u64,
) -> Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture> {
    super::parsed_capture(payload, capture_index)
        .filter(|capture| capture.status == ParseResultStatus::Success)
}

fn pattern_info(pattern_index: u64, pattern: &str) -> Option<(&'static str, &'static str)> {
    match (pattern_index, pattern.trim()) {
        (0, HIDE_PATTERN) => Some(("hide-entities", HIDE_PATTERN)),
        (1, REVEAL_PATTERN) => Some(("reveal-entities", REVEAL_PATTERN)),
        _ => None,
    }
}

fn viewer_source(
    pattern_index: u64,
    source_class: Option<&str>,
    explicit: bool,
) -> Option<ViewerSource> {
    if explicit {
        return Some(ViewerSource::Explicit);
    }
    if pattern_index == 0 {
        // A missing viewer means Bukkit.getOnlinePlayers() for hide.
        return Some(ViewerSource::AllOnlinePlayers);
    }
    match source_class {
        // Skript's reveal-special case forwards ExprHiddenPlayers.getViewers().
        Some(HIDDEN_PLAYERS_CLASS) => Some(ViewerSource::HiddenPlayersExpression),
        // A known non-special source uses Bukkit.getOnlinePlayers() at execution time.
        Some(_) => Some(ViewerSource::AllOnlinePlayers),
        None => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerSource {
    Explicit,
    HiddenPlayersExpression,
    AllOnlinePlayers,
}

impl ViewerSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit-viewers",
            Self::HiddenPlayersExpression => "hidden-players-expression",
            Self::AllOnlinePlayers => "all-online-players",
        }
    }
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
    use super::{HIDDEN_PLAYERS_CLASS, REVEAL_PATTERN, ViewerSource, pattern_info, viewer_source};

    #[test]
    fn hide_without_viewers_uses_all_online_players() {
        assert_eq!(
            classify_viewer(0, None, false),
            ViewerSource::AllOnlinePlayers
        );
    }

    #[test]
    fn reveal_hidden_players_forwards_the_expression_viewers() {
        assert_eq!(
            classify_viewer(1, Some(HIDDEN_PLAYERS_CLASS), false),
            ViewerSource::HiddenPlayersExpression
        );
    }

    #[test]
    fn an_explicit_viewer_expression_wins_over_the_default() {
        assert_eq!(
            classify_viewer(1, Some(HIDDEN_PLAYERS_CLASS), true),
            ViewerSource::Explicit
        );
    }

    #[test]
    fn a_pattern_index_is_only_valid_for_its_native_pattern() {
        assert!(pattern_info(1, REVEAL_PATTERN).is_some());
        assert!(pattern_info(0, REVEAL_PATTERN).is_none());
    }

    fn classify_viewer(
        pattern_index: u64,
        source_class: Option<&str>,
        explicit: bool,
    ) -> ViewerSource {
        viewer_source(pattern_index, source_class, explicit).expect("viewer source is known")
    }

    #[test]
    fn reveal_without_source_class_is_unresolved() {
        assert_eq!(viewer_source(1, None, false), None);
    }
}
