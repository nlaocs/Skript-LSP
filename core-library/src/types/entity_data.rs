use super::registered_literal::candidate_from_option;
use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLiteralOption,
    ExpressionPayload,
};

const ENTITY_DATA: &str = "ch.njol.skript.entity.EntityData";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.entity-data",
    classes: &[ENTITY_DATA],
    parse,
    unresolved: None,
    all_type_options: false,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }

    let literal = crate::language::strip_indefinite_article(text);
    let literal_start = payload
        .remaining
        .start
        .checked_add(u64::try_from(text.len() - literal.len()).ok()?)?;
    let mut parsed = parse_without_indefinite_article(payload, literal, literal_start, end)?;
    parsed.range.start = payload.remaining.start;
    Some(parsed)
}

/// Parses an entity description after a containing Type consumed its own prefix.
pub(super) fn parse_without_indefinite_article(
    payload: &ExpressionPayload,
    text: &str,
    start: u64,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }

    let java_trim = crate::runtime::skript_at_least_patch(2, 10, 2).unwrap_or(true);
    let without_leading = if java_trim {
        text.trim_start_matches(|character| character <= '\u{20}')
    } else {
        text
    };
    let literal = if java_trim {
        without_leading.trim_end_matches(|character| character <= '\u{20}')
    } else {
        without_leading
    };
    if literal.trim() != literal {
        return None;
    }
    let literal_start = start.checked_add((text.len() - without_leading.len()) as u64)?;
    let literal_end = end.checked_sub((without_leading.len() - literal.len()) as u64)?;

    if let Some(option) = payload
        .literal_options
        .iter()
        .filter(|option| {
            option.range.start == literal_start
                && option.range.end == literal_end
                && option.class_name == ENTITY_DATA
        })
        .min_by_key(|option| option.type_parse_order)
    {
        return Some(candidate_from_entity_option(option, start, end));
    }

    let active_type = payload
        .active_type
        .as_ref()
        .filter(|active| active.class_name == ENTITY_DATA)
        .or_else(|| {
            payload
                .type_options
                .iter()
                .filter(|option| option.class_name == ENTITY_DATA)
                .min_by_key(|option| option.type_parse_order)
        })?;
    let matched = registered_pattern_match(&active_type.registration_id, literal)?;

    Some(candidate_from_registration(matched, start, end))
}

fn candidate_from_entity_option(
    option: &ExpressionLiteralOption,
    start: u64,
    end: u64,
) -> ExpressionLeafCandidate {
    let mut parsed = candidate_from_option(option, "core.literal.entity-data", start, end);
    if let Some(represented_class) = option.represented_class.as_deref() {
        parsed
            .metadata
            .push(metadata("entity-class", represented_class));
    }
    parsed.metadata.push(metadata(
        "entity-plural",
        if option.plural { "true" } else { "false" },
    ));
    parsed
}

fn candidate_from_registration(
    matched: crate::nlaocs::skript_parser_addon::catalog_data::RegisteredTypePatternResult,
    start: u64,
    end: u64,
) -> ExpressionLeafCandidate {
    let mut parsed = candidate(
        "core.literal.entity-data",
        ExpressionLeafKind::Literal,
        start,
        end,
        ENTITY_DATA,
        DynamicMultiplicity::Single,
    );
    parsed.metadata = vec![
        metadata("entity-class", &matched.represented_class),
        metadata("entity-data-class", &matched.data_class),
        metadata(
            "entity-plural",
            if matched.tags.iter().any(|tag| tag == "plural") || matched.mark & 0x3 == 1 {
                "true"
            } else {
                "false"
            },
        ),
        metadata("entity-pattern", &matched.pattern),
        metadata("entity-source", "ssg.registered-parser-pattern"),
    ];
    if let Some(code_name) = matched.source_code_name.as_deref() {
        parsed
            .metadata
            .push(metadata("entity-code-name", code_name));
    }
    parsed
}

#[cfg(target_arch = "wasm32")]
fn registered_pattern_match(
    registration_id: &str,
    input: &str,
) -> Option<crate::nlaocs::skript_parser_addon::catalog_data::RegisteredTypePatternResult> {
    crate::nlaocs::skript_parser_addon::catalog_data::registered_type_pattern_match(
        registration_id,
        input,
    )
    .ok()
    .flatten()
}

#[cfg(not(target_arch = "wasm32"))]
fn registered_pattern_match(
    _registration_id: &str,
    _input: &str,
) -> Option<crate::nlaocs::skript_parser_addon::catalog_data::RegisteredTypePatternResult> {
    None
}
