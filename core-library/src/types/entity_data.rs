use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals
        || !payload
            .expected_types
            .iter()
            .any(|expected| expected.class_name == "ch.njol.skript.entity.EntityData")
        || !matches!(text.to_ascii_lowercase().as_str(), "player" | "players")
    {
        return None;
    }
    let mut candidate = candidate(
        "core.literal.entity-data",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        "ch.njol.skript.entity.EntityData",
        DynamicMultiplicity::Single,
    );
    candidate.metadata = vec![
        metadata("entity-class", "org.bukkit.entity.Player"),
        metadata(
            "entity-plural",
            if text.eq_ignore_ascii_case("players") {
                "true"
            } else {
                "false"
            },
        ),
    ];
    Some(candidate)
}
