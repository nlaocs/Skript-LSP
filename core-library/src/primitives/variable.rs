use crate::expression_candidates::candidate;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_expressions
        || text.len() < 3
        || !text.starts_with('{')
        || !text.ends_with('}')
        || text[1..text.len() - 1].trim().is_empty()
    {
        return None;
    }
    Some(candidate(
        "core.variable",
        ExpressionLeafKind::Variable,
        payload.remaining.start,
        end,
        payload
            .expected_types
            .first()
            .map_or("java.lang.Object", |expected| expected.class_name.as_str()),
        if text[1..text.len() - 1].trim_end().ends_with("::*") {
            DynamicMultiplicity::Multiple
        } else {
            DynamicMultiplicity::Single
        },
    ))
}
