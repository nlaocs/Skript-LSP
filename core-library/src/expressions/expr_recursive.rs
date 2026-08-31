use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprRecursive";
const HANDLER_ID: &str = "core.expression.expr-recursive";
const KEY_PROVIDER: &str = "expression.capability.key-provider";
const NESTED_STRUCTURES: &str = "expression.capability.nested-structures";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_recursive(payload))
}

fn resolve_recursive(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject("recursive Expression requires a source".to_owned());
    };
    if metadata_value(&source.metadata, NESTED_STRUCTURES) != Some("true") {
        return SemanticResolution::Reject(
            "the source Expression does not support nested structures".to_owned(),
        );
    }
    let Some(return_type) = source.return_type.clone() else {
        return SemanticResolution::Unresolved {
            reason: "recursive Expression source has no resolved return type".to_owned(),
            metadata: vec![metadata("semantic-mode", "recursive-values")],
        };
    };
    let possible_return_types = if source.possible_return_types.is_empty() {
        vec![return_type.clone()]
    } else {
        source.possible_return_types.clone()
    };
    let mut output_metadata = vec![
        metadata("semantic-mode", "recursive-values"),
        metadata(NESTED_STRUCTURES, "true"),
    ];
    if metadata_value(&source.metadata, KEY_PROVIDER) == Some("true") {
        output_metadata.push(metadata(KEY_PROVIDER, "true"));
    }
    SemanticResolution::Resolved {
        return_type,
        possible_return_types,
        possible_return_types_state: source.possible_return_types_state,
        multiplicity: source.multiplicity.unwrap_or(DynamicMultiplicity::Multiple),
        metadata: output_metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::{KEY_PROVIDER, NESTED_STRUCTURES};

    #[test]
    fn recursive_capabilities_use_shared_metadata_keys() {
        assert_eq!(KEY_PROVIDER, "expression.capability.key-provider");
        assert_eq!(NESTED_STRUCTURES, "expression.capability.nested-structures");
    }
}
