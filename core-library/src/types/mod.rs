mod boolean;
mod class_info;
mod entity_data;
mod item_type;
mod number;
mod registered_literal;
mod string_literal;

use crate::nlaocs::skript_parser_addon::types::{ExpressionLeafCandidate, ExpressionPayload};

pub(crate) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    string_literal::parse(payload, text, end)
        .or_else(|| number::parse(payload, text, end))
        .or_else(|| boolean::parse(payload, text, end))
        .or_else(|| item_type::parse(payload, text, end))
        // A ClassInfo expression must retain the selected type's metadata for
        // dynamic expressions such as ExprParse. A finite type literal with
        // the same spelling is still a value, so it must be considered only
        // after the ClassInfo-specific parser has had the opportunity to run.
        .or_else(|| class_info::parse(payload, text, end))
        .or_else(|| registered_literal::parse(payload, end))
        .or_else(|| entity_data::parse(payload, text, end))
}
