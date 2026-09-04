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
};
use crate::{catalog, catalog::TypeRelation};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    vec![entity_type::handler()]
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
    if let Some(active_type) = &payload.active_type {
        return crate::runtime::handler_matches(
            entity_type::HANDLER_ID,
            &active_type.registration_id,
        )
        .then(|| entity_type::parse(payload, text, end))
        .flatten();
    }
    let candidates = [
        ("string", string_literal::parse(payload, text, end)),
        ("number", number::parse(payload, text, end)),
        ("boolean", boolean::parse(payload, text, end)),
        ("itemtype", item_type::parse(payload, text, end)),
        ("entitydata", entity_data::parse(payload, text, end)),
        (
            "enchantmenttype",
            enchantment_type::parse(payload, text, end),
        ),
        ("timespan", timespan::parse(payload, text, end)),
        // ClassInfo parses a type name into a ClassInfo value. Its ordering is
        // therefore the `classinfo` ClassInfo registration, not the order of
        // the type named by the input.
        ("classinfo", class_info::parse(payload, text, end)),
        ("", registered_literal::parse(payload, end)),
    ];
    candidates
        .into_iter()
        .enumerate()
        .filter_map(|(fallback_order, (code_name, candidate))| {
            let candidate = candidate?;
            let code_name = if code_name.is_empty() {
                candidate
                    .metadata
                    .iter()
                    .find(|entry| entry.key == "type-code-name")
                    .map(|entry| entry.value.as_str())
                    .unwrap_or("")
            } else {
                code_name
            };
            let compatibility = candidate_compatibility(payload, &candidate)?;
            Some((
                compatibility,
                type_parse_order(payload, code_name, &candidate),
                fallback_order,
                candidate,
            ))
        })
        .min_by_key(|(compatibility, type_parse_order, fallback_order, _)| {
            (*compatibility, *type_parse_order, *fallback_order)
        })
        .map(|(_, _, _, candidate)| candidate)
}

fn candidate_compatibility(
    payload: &ExpressionPayload,
    candidate: &ExpressionLeafCandidate,
) -> Option<u8> {
    let return_type = candidate.return_type.as_deref()?;
    if payload.expected_types.is_empty() {
        return Some(0);
    }
    let mut unknown = false;
    for expected in &payload.expected_types {
        if return_type == expected.class_name {
            return Some(0);
        }
        match catalog::is_class_assignable(return_type, &expected.class_name)
            .unwrap_or(TypeRelation::Unknown)
        {
            TypeRelation::Compatible => return Some(0),
            TypeRelation::Unknown => unknown = true,
            TypeRelation::Incompatible => {}
        }
    }
    unknown.then_some(1)
}

fn type_parse_order(
    payload: &ExpressionPayload,
    code_name: &str,
    candidate: &ExpressionLeafCandidate,
) -> u64 {
    payload
        .type_options
        .iter()
        .filter(|option| option.code_name == code_name)
        .map(|option| option.type_parse_order)
        .chain(
            payload
                .literal_options
                .iter()
                .filter(|option| {
                    option.code_name == code_name
                        && option.range.start >= candidate.range.start
                        && option.range.end <= candidate.range.end
                })
                .map(|option| option.type_parse_order),
        )
        .min()
        .unwrap_or(u64::MAX)
}
