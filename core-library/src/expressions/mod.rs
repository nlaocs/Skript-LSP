mod context_validation;
pub(crate) mod event_value_expression;
mod expr_all_banned_entries;
mod expr_any_of;
mod expr_argument;
mod expr_attacked;
mod expr_cmd_cooldown_info;
mod expr_custom_model_data;
mod expr_default_value;
mod expr_difference;
mod expr_element;
mod expr_entities;
mod expr_entity;
mod expr_event_expression;
mod expr_except;
mod expr_filter;
mod expr_from_uuid;
mod expr_indices;
mod expr_indices_of_value;
mod expr_input;
mod expr_inventory;
mod expr_inventory_info;
mod expr_inventory_slot;
mod expr_items;
mod expr_join_split;
mod expr_keyed;
mod expr_last_spawned_entity;
mod expr_length;
mod expr_loop_value;
mod expr_midpoint;
mod expr_named;
mod expr_parse;
mod expr_potion_effect;
mod expr_random;
mod expr_random_character;
mod expr_random_number;
mod expr_recursive;
mod expr_reversed_list;
mod expr_sec_create_world_border;
mod expr_sets;
mod expr_shuffled_list;
mod expr_sorted_list;
mod expr_ternary;
mod expr_time_state;
mod expr_transform;
mod expr_type_of;
mod expr_value_within;
mod expr_whether;
mod expr_x_of;
mod prop_expr_amount;
mod prop_expr_custom_name;
mod prop_expr_name;
mod prop_expr_number;
mod prop_expr_progress;
mod prop_expr_scale;
mod prop_expr_size;
mod prop_expr_style;
mod prop_expr_title;
mod prop_expr_value_of;
mod prop_expr_viewers;
mod prop_expr_wxyz;
mod property;
mod simple_literal;
mod wrapper_expression;

use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, DynamicMultiplicity, ExpressionPossibleReturnTypesState, MetadataEntry,
    RegisteredExpressionChild, RegisteredExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use crate::runtime;
use parser_wasm::REGISTERED_CONTEXT_ALL_TYPE_OPTIONS;

pub(crate) enum SemanticResolution {
    Resolved {
        return_type: String,
        possible_return_types: Vec<String>,
        possible_return_types_state: ExpressionPossibleReturnTypesState,
        multiplicity: DynamicMultiplicity,
        metadata: Vec<MetadataEntry>,
    },
    Unresolved {
        reason: String,
        metadata: Vec<MetadataEntry>,
    },
    Reject(String),
}

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    context_validation::register(&mut handlers);
    event_value_expression::register(&mut handlers);
    expr_all_banned_entries::register(&mut handlers);
    expr_argument::register(&mut handlers);
    expr_attacked::register(&mut handlers);
    expr_any_of::register(&mut handlers);
    expr_cmd_cooldown_info::register(&mut handlers);
    expr_custom_model_data::register(&mut handlers);
    expr_default_value::register(&mut handlers);
    expr_difference::register(&mut handlers);
    expr_entities::register(&mut handlers);
    expr_entity::register(&mut handlers);
    expr_event_expression::register(&mut handlers);
    expr_except::register(&mut handlers);
    expr_from_uuid::register(&mut handlers);
    expr_filter::register(&mut handlers);
    expr_element::register(&mut handlers);
    expr_inventory_slot::register(&mut handlers);
    expr_input::register(&mut handlers);
    expr_indices::register(&mut handlers);
    expr_indices_of_value::register(&mut handlers);
    expr_inventory::register(&mut handlers);
    expr_inventory_info::register(&mut handlers);
    expr_join_split::register(&mut handlers);
    expr_keyed::register(&mut handlers);
    expr_last_spawned_entity::register(&mut handlers);
    expr_length::register(&mut handlers);
    expr_loop_value::register(&mut handlers);
    expr_midpoint::register(&mut handlers);
    expr_named::register(&mut handlers);
    expr_items::register(&mut handlers);
    expr_parse::register(&mut handlers);
    expr_potion_effect::register(&mut handlers);
    expr_random::register(&mut handlers);
    expr_random_character::register(&mut handlers);
    expr_random_number::register(&mut handlers);
    expr_recursive::register(&mut handlers);
    expr_reversed_list::register(&mut handlers);
    expr_sec_create_world_border::register(&mut handlers);
    expr_sets::register(&mut handlers);
    expr_shuffled_list::register(&mut handlers);
    expr_sorted_list::register(&mut handlers);
    expr_ternary::register(&mut handlers);
    expr_time_state::register(&mut handlers);
    expr_transform::register(&mut handlers);
    expr_type_of::register(&mut handlers);
    expr_value_within::register(&mut handlers);
    expr_whether::register(&mut handlers);
    expr_x_of::register(&mut handlers);
    prop_expr_amount::register(&mut handlers);
    prop_expr_custom_name::register(&mut handlers);
    prop_expr_name::register(&mut handlers);
    prop_expr_number::register(&mut handlers);
    prop_expr_progress::register(&mut handlers);
    prop_expr_scale::register(&mut handlers);
    prop_expr_size::register(&mut handlers);
    prop_expr_style::register(&mut handlers);
    prop_expr_title::register(&mut handlers);
    prop_expr_value_of::register(&mut handlers);
    prop_expr_viewers::register(&mut handlers);
    prop_expr_wxyz::register(&mut handlers);
    simple_literal::register(&mut handlers);
    wrapper_expression::register(&mut handlers);
    handlers
}

pub(crate) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    context_validation::resolve(payload)
        .or_else(|| event_value_expression::resolve(payload))
        .or_else(|| expr_all_banned_entries::resolve(payload))
        .or_else(|| expr_argument::resolve(payload))
        .or_else(|| expr_attacked::resolve(payload))
        .or_else(|| expr_any_of::resolve(payload))
        .or_else(|| expr_cmd_cooldown_info::resolve(payload))
        .or_else(|| expr_custom_model_data::resolve(payload))
        .or_else(|| expr_default_value::resolve(payload))
        .or_else(|| expr_difference::resolve(payload))
        .or_else(|| expr_entities::resolve(payload))
        .or_else(|| expr_entity::resolve(payload))
        .or_else(|| expr_event_expression::resolve(payload))
        .or_else(|| expr_except::resolve(payload))
        .or_else(|| expr_from_uuid::resolve(payload))
        .or_else(|| expr_filter::resolve(payload))
        .or_else(|| expr_element::resolve(payload))
        .or_else(|| expr_inventory_slot::resolve(payload))
        .or_else(|| expr_input::resolve(payload))
        .or_else(|| expr_indices::resolve(payload))
        .or_else(|| expr_indices_of_value::resolve(payload))
        .or_else(|| expr_inventory::resolve(payload))
        .or_else(|| expr_inventory_info::resolve(payload))
        .or_else(|| expr_join_split::resolve(payload))
        .or_else(|| expr_keyed::resolve(payload))
        .or_else(|| expr_last_spawned_entity::resolve(payload))
        .or_else(|| expr_length::resolve(payload))
        .or_else(|| expr_loop_value::resolve(payload))
        .or_else(|| expr_midpoint::resolve(payload))
        .or_else(|| expr_named::resolve(payload))
        .or_else(|| expr_items::resolve(payload))
        .or_else(|| expr_parse::resolve(payload))
        .or_else(|| expr_potion_effect::resolve(payload))
        .or_else(|| expr_random::resolve(payload))
        .or_else(|| expr_random_character::resolve(payload))
        .or_else(|| expr_random_number::resolve(payload))
        .or_else(|| expr_recursive::resolve(payload))
        .or_else(|| expr_reversed_list::resolve(payload))
        .or_else(|| expr_sec_create_world_border::resolve(payload))
        .or_else(|| expr_sets::resolve(payload))
        .or_else(|| expr_shuffled_list::resolve(payload))
        .or_else(|| expr_sorted_list::resolve(payload))
        .or_else(|| expr_ternary::resolve(payload))
        .or_else(|| expr_time_state::resolve(payload))
        .or_else(|| expr_transform::resolve(payload))
        .or_else(|| expr_type_of::resolve(payload))
        .or_else(|| expr_value_within::resolve(payload))
        .or_else(|| expr_whether::resolve(payload))
        .or_else(|| expr_x_of::resolve(payload))
        .or_else(|| prop_expr_amount::resolve(payload))
        .or_else(|| prop_expr_custom_name::resolve(payload))
        .or_else(|| prop_expr_name::resolve(payload))
        .or_else(|| prop_expr_number::resolve(payload))
        .or_else(|| prop_expr_progress::resolve(payload))
        .or_else(|| prop_expr_scale::resolve(payload))
        .or_else(|| prop_expr_size::resolve(payload))
        .or_else(|| prop_expr_style::resolve(payload))
        .or_else(|| prop_expr_title::resolve(payload))
        .or_else(|| prop_expr_value_of::resolve(payload))
        .or_else(|| prop_expr_viewers::resolve(payload))
        .or_else(|| prop_expr_wxyz::resolve(payload))
        .or_else(|| simple_literal::resolve(payload))
        .or_else(|| wrapper_expression::resolve(payload))
}

fn register_handler(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
    capture_parsers: Vec<CaptureParserBinding>,
) {
    register_handler_with_context(
        handlers,
        handler_id,
        class_suffix,
        capture_parsers,
        Vec::new(),
    );
}

fn register_handler_targets(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffixes: &[&str],
) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Expression,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets: class_suffixes
            .iter()
            .map(|suffix| RegisteredSyntaxHandlerTarget::ClassSuffix((*suffix).to_owned()))
            .collect(),
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

fn register_handler_with_all_type_options(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
    capture_parsers: Vec<CaptureParserBinding>,
) {
    register_handler_with_context(
        handlers,
        handler_id,
        class_suffix,
        capture_parsers,
        vec![REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()],
    );
}

fn register_handler_with_context(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
    capture_parsers: Vec<CaptureParserBinding>,
    context_requirements: Vec<String>,
) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Expression,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
            class_suffix.to_owned(),
        )],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers,
        context_requirements,
    });
}

fn capture_parser(capture_index: u64, parser_id: &str) -> CaptureParserBinding {
    CaptureParserBinding {
        capture_index,
        parser_id: parser_id.to_owned(),
        required: true,
        options: Vec::new(),
    }
}

fn matches(payload: &RegisteredExpressionPayload, handler_id: &str) -> bool {
    #[cfg(test)]
    let class_matches = {
        // Direct semantic unit tests do not run through HostProfile. Keep
        // those tests independent from process-global runtime initialization.
        let handler_name = handler_id.rsplit('.').next().unwrap_or(handler_id);
        let class_name = payload.element_class.rsplit('.').next().unwrap_or_default();
        normalize_test_name(handler_name) == normalize_test_name(class_name)
    };
    #[cfg(test)]
    if payload.definition_id == "expression:test" {
        return class_matches;
    }
    if runtime::handler_matches(handler_id, &payload.registration_id) {
        return true;
    }
    #[cfg(test)]
    {
        class_matches
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[cfg(test)]
fn normalize_test_name(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn resolved(
    return_type: &str,
    multiplicity: DynamicMultiplicity,
    mode: &str,
) -> SemanticResolution {
    resolved_with_metadata(
        return_type.to_owned(),
        multiplicity,
        vec![metadata("semantic-mode", mode)],
    )
}

fn resolved_with_metadata(
    return_type: String,
    multiplicity: DynamicMultiplicity,
    metadata: Vec<MetadataEntry>,
) -> SemanticResolution {
    let possible_return_types = vec![return_type.clone()];
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        metadata,
    )
}

fn resolved_with_possible_types(
    return_type: String,
    possible_return_types: Vec<String>,
    possible_return_types_state: ExpressionPossibleReturnTypesState,
    multiplicity: DynamicMultiplicity,
    metadata: Vec<MetadataEntry>,
) -> SemanticResolution {
    SemanticResolution::Resolved {
        return_type,
        possible_return_types,
        possible_return_types_state,
        multiplicity,
        metadata,
    }
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

fn set_metadata(metadata: &mut Vec<MetadataEntry>, key: &str, value: &str) {
    if let Some(entry) = metadata.iter_mut().find(|entry| entry.key == key) {
        entry.value = value.to_owned();
    } else {
        metadata.push(self::metadata(key, value));
    }
}

fn metadata_value<'a>(metadata: &'a [MetadataEntry], key: &str) -> Option<&'a str> {
    metadata
        .iter()
        .find(|entry| {
            entry.key == key
                || entry
                    .key
                    .rsplit_once('/')
                    .is_some_and(|(_, suffix)| suffix == key)
        })
        .map(|entry| entry.value.as_str())
}

fn optional_integer_amount_multiplicity(
    children: &[RegisteredExpressionChild],
) -> Option<DynamicMultiplicity> {
    match children {
        [_, _] => Some(DynamicMultiplicity::Single),
        [amount, _, _] => Some(
            if amount.kind == "literal"
                && matches!(
                    amount.return_type.as_deref(),
                    Some("java.lang.Integer" | "java.lang.Long")
                )
                && amount.text.parse::<i64>() == Ok(1)
            {
                DynamicMultiplicity::Single
            } else {
                DynamicMultiplicity::Multiple
            },
        ),
        _ => None,
    }
}
