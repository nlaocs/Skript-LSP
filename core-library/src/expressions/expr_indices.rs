use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprIndices";
const HANDLER_ID: &str = "core.expression.expr-indices";
const KEY_PROVIDER: &str = "expression.capability.key-provider";
const NESTED_STRUCTURES: &str = "expression.capability.nested-structures";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_indices(payload))
}

fn resolve_indices(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject("indices Expression requires a source".to_owned());
    };
    if metadata_value(&source.metadata, KEY_PROVIDER) != Some("true") {
        return SemanticResolution::Reject(
            "the indices Expression may only be used with keyed Expressions".to_owned(),
        );
    }
    let mut output_metadata = vec![metadata("semantic-mode", "list-indices")];
    if metadata_value(&source.metadata, NESTED_STRUCTURES) == Some("true") {
        output_metadata.push(metadata(NESTED_STRUCTURES, "true"));
    }
    SemanticResolution::Resolved {
        return_type: "java.lang.String".to_owned(),
        possible_return_types: vec!["java.lang.String".to_owned()],
        possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
        multiplicity: DynamicMultiplicity::Multiple,
        metadata: output_metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::{CLASS_SUFFIX, HANDLER_ID};

    #[test]
    fn targets_the_native_indices_expression() {
        assert_eq!(CLASS_SUFFIX, ".ExprIndices");
        assert_eq!(HANDLER_ID, "core.expression.expr-indices");
    }
}
