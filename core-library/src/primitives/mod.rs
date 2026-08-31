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

pub(crate) fn is_valid_variable_name_body(body: &str, allow_list_variable: bool) -> bool {
    variable::is_valid_variable_name_body(body, allow_list_variable)
}
