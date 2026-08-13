mod expr_element;
mod expr_entities;
mod expr_inventory_slot;
mod expr_parse;
mod expr_random;
mod expr_sets;
mod expr_ternary;
mod expr_whether;
mod prop_expr_amount;
mod prop_expr_custom_name;
mod prop_expr_name;
mod prop_expr_number;
mod prop_expr_scale;
mod prop_expr_size;
mod prop_expr_value_of;
mod prop_expr_wxyz;
mod property;

use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, MetadataEntry, RegisteredCaptureKind, RegisteredExpressionPayload,
    RegisteredSyntaxHandler, SyntaxKind,
};

pub(crate) enum SemanticResolution {
    Resolved {
        return_type: String,
        multiplicity: DynamicMultiplicity,
        metadata: Vec<MetadataEntry>,
    },
    Reject(String),
}

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    expr_entities::register(&mut handlers);
    expr_element::register(&mut handlers);
    expr_inventory_slot::register(&mut handlers);
    expr_parse::register(&mut handlers);
    expr_random::register(&mut handlers);
    expr_sets::register(&mut handlers);
    expr_ternary::register(&mut handlers);
    expr_whether::register(&mut handlers);
    prop_expr_amount::register(&mut handlers);
    prop_expr_custom_name::register(&mut handlers);
    prop_expr_name::register(&mut handlers);
    prop_expr_number::register(&mut handlers);
    prop_expr_scale::register(&mut handlers);
    prop_expr_size::register(&mut handlers);
    prop_expr_value_of::register(&mut handlers);
    prop_expr_wxyz::register(&mut handlers);
    handlers
}

pub(crate) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    expr_entities::resolve(payload)
        .or_else(|| expr_element::resolve(payload))
        .or_else(|| expr_inventory_slot::resolve(payload))
        .or_else(|| expr_parse::resolve(payload))
        .or_else(|| expr_random::resolve(payload))
        .or_else(|| expr_sets::resolve(payload))
        .or_else(|| expr_ternary::resolve(payload))
        .or_else(|| expr_whether::resolve(payload))
        .or_else(|| prop_expr_amount::resolve(payload))
        .or_else(|| prop_expr_custom_name::resolve(payload))
        .or_else(|| prop_expr_name::resolve(payload))
        .or_else(|| prop_expr_number::resolve(payload))
        .or_else(|| prop_expr_scale::resolve(payload))
        .or_else(|| prop_expr_size::resolve(payload))
        .or_else(|| prop_expr_value_of::resolve(payload))
        .or_else(|| prop_expr_wxyz::resolve(payload))
}

fn register_handler(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    class_suffix: &str,
    regex_captures: Vec<RegisteredCaptureKind>,
) {
    handlers.push(RegisteredSyntaxHandler {
        kind: SyntaxKind::Expression,
        class_suffix: class_suffix.to_owned(),
        regex_captures,
    });
}

fn matches(payload: &RegisteredExpressionPayload, class_suffix: &str) -> bool {
    payload.element_class.ends_with(class_suffix)
}

fn resolved(
    return_type: &str,
    multiplicity: DynamicMultiplicity,
    mode: &str,
) -> SemanticResolution {
    SemanticResolution::Resolved {
        return_type: return_type.to_owned(),
        multiplicity,
        metadata: vec![metadata("semantic-mode", mode)],
    }
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn metadata_value<'a>(metadata: &'a [MetadataEntry], key: &str) -> Option<&'a str> {
    metadata
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}
