mod boolean;
mod class_info;
mod enchantment_type;
mod entity_data;
mod entity_type;
mod item_type;
mod number;
mod registered_literal;
mod string_literal;
mod timespan;

use crate::nlaocs::skript_parser_addon::types::{
    ExpressionLeafCandidate, ExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};

pub(super) struct TypeParser {
    id: &'static str,
    classes: &'static [&'static str],
    parse: fn(&ExpressionPayload, &str, u64) -> Option<ExpressionLeafCandidate>,
    all_type_options: bool,
}

const PARSERS: &[TypeParser] = &[
    string_literal::PARSER,
    number::PARSER,
    boolean::PARSER,
    item_type::PARSER,
    entity_data::PARSER,
    entity_type::PARSER,
    enchantment_type::PARSER,
    timespan::PARSER,
    class_info::PARSER,
];

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    PARSERS
        .iter()
        .map(|parser| RegisteredSyntaxHandler {
            handler_id: parser.id.to_owned(),
            kind: SyntaxKind::Type,
            targets: parser
                .classes
                .iter()
                .map(|class| RegisteredSyntaxHandlerTarget::ClassSuffix((*class).to_owned()))
                .collect(),
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: if parser.all_type_options {
                vec![parser_wasm::REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()]
            } else {
                Vec::new()
            },
        })
        .collect()
}

pub(crate) fn match_type_option<'a>(
    name: &str,
    options: &'a [crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> Option<(
    &'a crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption,
    bool,
)> {
    class_info::type_option(name, options)
}

pub(crate) fn match_user_type_option(
    name: &str,
    _options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> Option<(
    crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption,
    bool,
)> {
    #[cfg(target_arch = "wasm32")]
    {
        return crate::nlaocs::skript_parser_addon::catalog_data::type_for_user_input(name)
            .ok()
            .flatten()
            .map(|option| {
                let plural = class_info::is_plural_user_input(name, &option);
                (option, plural)
            });
    }
    #[cfg(not(target_arch = "wasm32"))]
    class_info::user_type_option(name, _options).map(|(option, plural)| (option.clone(), plural))
}

pub(crate) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    payload.active_type.as_ref()?;
    let mut candidate = if let Some(parser) = standard_parser(payload) {
        (parser.parse)(payload, text, end)
    } else {
        registered_literal::parse(payload, end)
    }?;
    annotate(payload, &mut candidate);
    Some(candidate)
}

fn standard_parser(payload: &ExpressionPayload) -> Option<&'static TypeParser> {
    let active = payload.active_type.as_ref()?;
    PARSERS.iter().find(|parser| {
        if !parser.classes.contains(&active.class_name.as_str()) {
            return false;
        }
        // Direct unit fixtures bypass component initialization, never production IDs.
        #[cfg(test)]
        if active.registration_id.starts_with("type:test:") {
            return true;
        }
        crate::runtime::handler_matches(parser.id, &active.registration_id)
    })
}

pub(crate) fn parses_string(payload: &ExpressionPayload) -> bool {
    standard_parser(payload).is_some_and(|parser| parser.id == string_literal::PARSER.id)
}

pub(crate) fn annotate(payload: &ExpressionPayload, candidate: &mut ExpressionLeafCandidate) {
    let Some(active) = &payload.active_type else {
        return;
    };
    // These identify the parser's Type, not a ClassInfo value's target Type.
    candidate.metadata.extend([
        crate::expression_candidates::metadata("type-parser-definition-id", &active.definition_id),
        crate::expression_candidates::metadata(
            "type-parser-registration-id",
            &active.registration_id,
        ),
        crate::expression_candidates::metadata("type-parser-code-name", &active.code_name),
    ]);
}
