use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let value = match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => "true",
        "false" | "no" | "off" => "false",
        _ => return None,
    };
    let mut candidate = candidate(
        "core.literal.boolean",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        "java.lang.Boolean",
        DynamicMultiplicity::Single,
    );
    candidate.metadata.push(metadata("boolean-value", value));
    Some(candidate)
}
