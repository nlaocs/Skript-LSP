use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
    ExpressionTypeOption,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let (option, plural) = type_option(text, &payload.type_options)?;
    let mut candidate = candidate(
        "core.literal.class-info",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        payload
            .expected_types
            .first()
            .map_or("ch.njol.skript.classes.ClassInfo", |expected| {
                expected.class_name.as_str()
            }),
        DynamicMultiplicity::Single,
    );
    candidate.metadata = vec![
        metadata("target-class", &option.class_name),
        metadata("type-code-name", &option.code_name),
        metadata("type-plural", if plural { "true" } else { "false" }),
        metadata(
            "has-parser",
            if option.has_parser { "true" } else { "false" },
        ),
        metadata(
            "has-supplier",
            if option.has_supplier { "true" } else { "false" },
        ),
    ];
    Some(candidate)
}

fn type_option<'a>(
    name: &str,
    options: &'a [ExpressionTypeOption],
) -> Option<(&'a ExpressionTypeOption, bool)> {
    let name = name.trim();
    options.iter().find_map(|option| {
        if name.eq_ignore_ascii_case(&option.plural) {
            Some((option, true))
        } else if name.eq_ignore_ascii_case(&option.code_name)
            || name.eq_ignore_ascii_case(&option.singular)
        {
            Some((option, false))
        } else {
            None
        }
    })
}
