//! Fixed declared bounds for authenticated property families without a more
//! specific semantic resolver. This does not certify getter effects or presence.
use super::{SemanticResolution,resolved};
use crate::nlaocs::skript_parser_addon::types::{
    ExpressionPossibleReturnTypesState, ExpressionReturnTypeState,
    RegisteredExpressionPayload, RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, SyntaxKind,
};
const HANDLER_ID: &str = "core.expression.fixed-property";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id:HANDLER_ID.into(),kind:SyntaxKind::Expression,
        phase:crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets:["org.skriptlang.skript.lang.properties.PropertyBaseExpression",
            "ch.njol.skript.expressions.base.SimplePropertyExpression",
            "ch.njol.skript.expressions.base.PropertyExpression"].into_iter()
            .map(|base|RegisteredSyntaxHandlerTarget::SuperClass(base.into())).collect(),
        pattern_indices:Vec::new(),pattern_sources:Vec::new(),required_tags:Vec::new(),
        forbidden_tags:Vec::new(),marks:Vec::new(),capture_parsers:Vec::new(),context_requirements:Vec::new(),
    });
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if !super::matches(payload,HANDLER_ID) { return None; }
    let class=fixed_bound(payload.return_type_state,payload.declared_return_type.as_deref(),
        &payload.possible_return_types,payload.possible_return_types_state)?;
    // This fallback never overwrites a preceding provider's different result.
    if fixed_bound(payload.return_type_state,payload.effective_return_type.as_deref(),
        &payload.effective_possible_return_types,payload.effective_possible_return_types_state)?!=class {
        return None;
    }
    let multiplicity=payload.effective_multiplicity.or(payload.declared_multiplicity)?;
    if !crate::catalog::standard_property_source(payload) { return None; }
    Some(resolved(class,multiplicity,"fixed-property"))
}

fn fixed_bound<'a>(state: ExpressionReturnTypeState, declared: Option<&'a str>,
    possible: &[String], completeness: ExpressionPossibleReturnTypesState) -> Option<&'a str> {
    let class=declared.filter(|class|!class.trim().is_empty() && *class!="java.lang.Object")?;
    (state==ExpressionReturnTypeState::Static && completeness==ExpressionPossibleReturnTypesState::Complete
        && possible.len()==1 && possible[0]==class).then_some(class)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_property_bound_does_not_promote_dynamic_partial_or_conflicting_metadata() {
        use ExpressionPossibleReturnTypesState::{Complete,Partial,Unresolved};
        let possible=vec!["fixture.Value".into()];
        assert_eq!(fixed_bound(ExpressionReturnTypeState::Static,Some("fixture.Value"),&possible,Complete),Some("fixture.Value"));
        for state in [Partial,Unresolved] { assert!(fixed_bound(ExpressionReturnTypeState::Static,Some("fixture.Value"),&possible,state).is_none()); }
        assert!(fixed_bound(ExpressionReturnTypeState::Dynamic,Some("fixture.Value"),&possible,Complete).is_none());
        for declared in [None,Some(""),Some("java.lang.Object"),Some("fixture.Other")] {
            assert!(fixed_bound(ExpressionReturnTypeState::Static,declared,&possible,Complete).is_none());
        }
        for possible in [vec![],vec!["fixture.Value".into(),"fixture.Other".into()]] {
            assert!(fixed_bound(ExpressionReturnTypeState::Static,Some("fixture.Value"),&possible,Complete).is_none());
        }
    }
}
