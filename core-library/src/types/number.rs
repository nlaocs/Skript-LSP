use crate::expression_candidates::candidate;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals
        || text.is_empty()
        || !text.parse::<f64>().is_ok_and(|value| value.is_finite())
    {
        return None;
    }
    Some(candidate(
        "core.literal.number",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        if text.contains(['.', 'e', 'E']) {
            "java.lang.Double"
        } else {
            "java.lang.Long"
        },
        DynamicMultiplicity::Single,
    ))
}
