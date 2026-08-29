use crate::expression_candidates::candidate;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals || text.len() < 2 || !text.starts_with('"') || !text.ends_with('"') {
        return None;
    }
    let mut chars = text[1..text.len() - 1].chars().peekable();
    while let Some(character) = chars.next() {
        if character == '"' && chars.next_if_eq(&'"').is_none() {
            return None;
        }
    }
    Some(candidate(
        "core.literal.string",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        "java.lang.String",
        DynamicMultiplicity::Single,
    ))
}
