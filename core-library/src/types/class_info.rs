use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
    ExpressionTypeOption,
};
use fancy_regex::Regex;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static USER_INPUT_PATTERNS: RefCell<HashMap<String, Option<Regex>>> =
        RefCell::new(HashMap::new());
}

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
        "ch.njol.skript.classes.ClassInfo",
        DynamicMultiplicity::Single,
    );
    candidate.metadata = vec![
        metadata("semantic-role", "target-type"),
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
    let exact = options.iter().find_map(|option| {
        if name.eq_ignore_ascii_case(&option.plural) {
            Some((option, true))
        } else if name.eq_ignore_ascii_case(&option.code_name)
            || name.eq_ignore_ascii_case(&option.singular)
        {
            Some((option, false))
        } else {
            None
        }
    });
    exact.or_else(|| {
        options.iter().find_map(|option| {
            option
                .user_input_patterns
                .iter()
                .any(|pattern| matches_user_input(pattern, name))
                .then_some((option, name.ends_with('s')))
        })
    })
}

fn matches_user_input(pattern: &str, input: &str) -> bool {
    USER_INPUT_PATTERNS.with(|patterns| {
        let mut patterns = patterns.borrow_mut();
        patterns
            .entry(pattern.to_owned())
            .or_insert_with(|| Regex::new(&format!("(?i)^(?:{pattern})$")).ok())
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(input).unwrap_or(false))
    })
}
