use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::catalog;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprExcept";
const HANDLER_ID: &str = "core.expression.expr-except";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_except(payload))
}

fn resolve_except(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let [source, _exclude] = payload.children.as_slice() else {
        return SemanticResolution::Reject(
            "except Expression requires a source and an excluded Expression".to_owned(),
        );
    };
    let Some(multiplicity) = source.multiplicity.or(payload.declared_multiplicity) else {
        return unresolved("except Expression source multiplicity is unresolved");
    };
    if !source_is_valid(multiplicity == DynamicMultiplicity::Single, &source.kind) {
        return SemanticResolution::Reject(
            "except Expression requires a list when the source is single-valued".to_owned(),
        );
    }

    let Some(return_type) = source
        .return_type
        .as_deref()
        .or(payload.declared_return_type.as_deref())
        .filter(|return_type| !return_type.is_empty() && *return_type != OBJECT)
        .map(str::to_owned)
    else {
        return unresolved("except Expression source return type is unresolved");
    };
    let possible_return_types = if source.possible_return_types.is_empty() {
        vec![return_type.clone()]
    } else {
        source.possible_return_types.clone()
    };
    let mut output_metadata = vec![metadata("semantic-mode", "except")];
    if let Ok(Some(contract)) = catalog::child_change_contract(source)
        && let Ok(contract) = catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        output_metadata.push(contract);
    }
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        source.possible_return_types_state,
        multiplicity,
        output_metadata,
    )
}

fn unresolved(reason: &str) -> SemanticResolution {
    SemanticResolution::Unresolved {
        reason: reason.to_owned(),
        metadata: vec![metadata("semantic-mode", "except")],
    }
}

fn source_is_valid(single: bool, kind: &str) -> bool {
    !single || kind == "expression-list"
}

#[cfg(test)]
mod tests {
    use super::source_is_valid;

    #[test]
    fn a_single_source_is_valid_only_when_it_is_an_expression_list() {
        assert!(source_is_valid(true, "expression-list"));
        assert!(!source_is_valid(true, "expression"));
        assert!(source_is_valid(false, "expression"));
    }
}
