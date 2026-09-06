use super::{SemanticResolution, matches, metadata, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};

const HANDLER_ID: &str = "core.expression.simple-literal";
const SUPER_CLASS: &str = "ch.njol.skript.lang.util.SimpleLiteral";

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
    let Some(return_type) = payload.declared_return_type.as_deref() else {
        return Some(SemanticResolution::Unresolved {
            reason: "SimpleLiteral return type is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "simple-literal")],
        });
    };
    Some(resolved(
        return_type,
        simple_literal_multiplicity(payload.declared_multiplicity),
        "simple-literal",
    ))
}

fn simple_literal_multiplicity(declared: Option<DynamicMultiplicity>) -> DynamicMultiplicity {
    // SimpleLiteral computes isSingle() from its data length and `and` flag.
    // Without constructor metadata, the official implementation can therefore
    // produce either one value or multiple values; Both is intentional here,
    // rather than a substitute for missing type information.
    declared.unwrap_or(DynamicMultiplicity::Both)
}

#[cfg(test)]
mod tests {
    use super::{HANDLER_ID, SUPER_CLASS, register, simple_literal_multiplicity};
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, RegisteredSyntaxHandlerTarget,
    };

    #[test]
    fn handler_targets_every_direct_simple_literal_subclass() {
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert_eq!(handlers[0].handler_id, HANDLER_ID);
        assert!(matches!(
            &handlers[0].targets[0],
            RegisteredSyntaxHandlerTarget::SuperClass(value) if value == SUPER_CLASS
        ));
    }

    #[test]
    fn missing_constructor_metadata_keeps_runtime_dependent_multiplicity() {
        assert_eq!(simple_literal_multiplicity(None), DynamicMultiplicity::Both);
        assert_eq!(
            simple_literal_multiplicity(Some(DynamicMultiplicity::Single)),
            DynamicMultiplicity::Single
        );
    }
}
