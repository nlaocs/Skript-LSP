use super::{SemanticResolution, matches, metadata, resolved_with_possible_types};
use crate::catalog;
use crate::nlaocs::skript_parser_addon::types::{
    ExpressionPossibleReturnTypesState, RegisteredExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};

const HANDLER_ID: &str = "core.expression.wrapper";
const SUPER_CLASS: &str = "ch.njol.skript.expressions.base.WrapperExpression";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Expression,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets: vec![RegisteredSyntaxHandlerTarget::SuperClass(
            SUPER_CLASS.to_owned(),
        )],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if !matches(payload, HANDLER_ID) {
        return None;
    }
    let child = payload.children.first();
    let declared_return_type = payload
        .declared_return_type
        .as_deref()
        .filter(|return_type| *return_type != OBJECT);
    let return_type = declared_return_type
        .or(payload.common_child_return_type.as_deref())
        .or_else(|| child.and_then(|child| child.return_type.as_deref()))?;
    let multiplicity = wrapper_multiplicity(
        child.and_then(|child| child.multiplicity),
        payload.declared_multiplicity,
    )?;
    let (possible_return_types, possible_return_types_state) = if declared_return_type.is_some() {
        let mut possible = payload.possible_return_types.clone();
        let state = if possible.is_empty() {
            possible.push(return_type.to_owned());
            ExpressionPossibleReturnTypesState::Complete
        } else {
            payload.possible_return_types_state
        };
        (possible, state)
    } else {
        child.map_or_else(
            || {
                (
                    vec![return_type.to_owned()],
                    ExpressionPossibleReturnTypesState::Complete,
                )
            },
            |child| {
                let mut possible = child.possible_return_types.clone();
                if possible.is_empty() {
                    possible.push(return_type.to_owned());
                }
                (possible, child.possible_return_types_state)
            },
        )
    };
    let mut output_metadata = vec![metadata("semantic-mode", "wrapper-expression")];
    if let Some(child) = child
        && let Ok(Some(contract)) = catalog::child_change_contract(child)
        && let Ok(contract) = catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        output_metadata.push(contract);
    }
    if child.is_some_and(|child| {
        child.metadata.iter().any(|entry| {
            entry
                .key
                .ends_with("expression.capability.nested-structures")
                && entry.value == "true"
        })
    }) {
        output_metadata.push(metadata("expression.capability.nested-structures", "true"));
    }
    Some(resolved_with_possible_types(
        return_type.to_owned(),
        possible_return_types,
        possible_return_types_state,
        multiplicity,
        output_metadata,
    ))
}

fn wrapper_multiplicity(
    child: Option<crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity>,
    declared: Option<crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity>,
) -> Option<crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity> {
    child.or(declared)
}

#[cfg(test)]
mod tests {
    use super::{HANDLER_ID, SUPER_CLASS, register, wrapper_multiplicity};
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, RegisteredSyntaxHandlerTarget,
    };

    #[test]
    fn handler_targets_the_native_wrapper_base_class() {
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert_eq!(handlers[0].handler_id, HANDLER_ID);
        assert!(matches!(
            &handlers[0].targets[0],
            RegisteredSyntaxHandlerTarget::SuperClass(value) if value == SUPER_CLASS
        ));
    }

    #[test]
    fn child_multiplicity_is_authoritative_for_wrappers() {
        assert_eq!(
            wrapper_multiplicity(
                Some(DynamicMultiplicity::Single),
                Some(DynamicMultiplicity::Multiple)
            ),
            Some(DynamicMultiplicity::Single)
        );
        assert_eq!(
            wrapper_multiplicity(
                Some(DynamicMultiplicity::Multiple),
                Some(DynamicMultiplicity::Single)
            ),
            Some(DynamicMultiplicity::Multiple)
        );
        assert_eq!(
            wrapper_multiplicity(None, Some(DynamicMultiplicity::Both)),
            Some(DynamicMultiplicity::Both)
        );
    }
}
