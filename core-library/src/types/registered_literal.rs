use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLiteralOption,
    ExpressionLiteralSource, ExpressionPayload,
};

pub(super) fn parse(payload: &ExpressionPayload, end: u64) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let active = payload.active_type.as_ref()?;
    let option = payload
        .literal_options
        .iter()
        .filter(|option| {
            option.code_name == active.code_name
                && option.class_name == active.class_name
                && option.type_parse_order == active.type_parse_order
        })
        .filter(|option| option.range.start == payload.remaining.start && option.range.end == end)
        .min_by_key(|option| option.type_parse_order)?;
    Some(candidate_from_option(
        option,
        "core.literal.type",
        option.range.start,
        option.range.end,
    ))
}

pub(super) fn candidate_from_option(
    option: &ExpressionLiteralOption,
    parser_id: &str,
    start: u64,
    end: u64,
) -> ExpressionLeafCandidate {
    let mut candidate = candidate(
        parser_id,
        ExpressionLeafKind::Literal,
        start,
        end,
        &option.class_name,
        DynamicMultiplicity::Single,
    );
    candidate.metadata = vec![
        metadata("type-code-name", &option.code_name),
        metadata("literal-canonical", &option.canonical_value),
        metadata("literal-range-start", &option.range.start.to_string()),
        metadata("literal-range-end", &option.range.end.to_string()),
        metadata(
            "literal-source",
            match option.source {
                ExpressionLiteralSource::ParserPattern => "parser-pattern",
                ExpressionLiteralSource::Supplier => "supplier",
                ExpressionLiteralSource::EnumConstant => "enum-constant",
                ExpressionLiteralSource::Alias => "alias",
            },
        ),
        metadata(
            "literal-plural",
            if option.plural { "true" } else { "false" },
        ),
        metadata("type-addon", &option.addon_name),
        metadata("type-addon-version", &option.addon_version),
    ];
    push_optional(
        &mut candidate.metadata,
        "type-parser-class",
        option.parser_class.as_deref(),
    );
    if !option.parse_contexts.is_empty() {
        candidate.metadata.push(metadata(
            "type-parse-contexts",
            &option.parse_contexts.join(","),
        ));
    }
    push_optional(
        &mut candidate.metadata,
        "literal-value-class",
        option.value_class.as_deref(),
    );
    push_optional(
        &mut candidate.metadata,
        "literal-represented-class",
        option.represented_class.as_deref(),
    );
    push_optional(
        &mut candidate.metadata,
        "literal-variable-name",
        option.variable_name.as_deref(),
    );
    push_optional(
        &mut candidate.metadata,
        "literal-debug-text",
        option.debug_text.as_deref(),
    );
    push_optional(
        &mut candidate.metadata,
        "literal-enum-constant",
        option.enum_constant.as_deref(),
    );
    if let Some(all) = option.alias_all {
        candidate
            .metadata
            .push(metadata("literal-alias-all", &all.to_string()));
    }
    if let Some(type_count) = option.alias_type_count {
        candidate.metadata.push(metadata(
            "literal-alias-type-count",
            &type_count.to_string(),
        ));
    }
    candidate
}

fn push_optional(
    metadata_entries: &mut Vec<crate::nlaocs::skript_parser_addon::types::MetadataEntry>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        metadata_entries.push(metadata(key, value));
    }
}
