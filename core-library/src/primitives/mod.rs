pub(crate) mod interpolation;
mod variable;

use crate::nlaocs::skript_parser_addon::types::{ExpressionLeafCandidate, ExpressionPayload};

pub(crate) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    variable::parse(payload, text, end)
}
