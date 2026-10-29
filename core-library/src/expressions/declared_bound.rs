//! Ordinary declaration fallback after the specific standard resolvers. It
//! neither validates arbitrary init callbacks nor certifies their side effects.
use super::{SemanticResolution,resolved_with_possible_types};
use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState,ExpressionReturnTypeState,RegisteredExpressionPayload};

pub(super) fn resolve(payload:&RegisteredExpressionPayload)->Option<SemanticResolution> {
    let class=payload.declared_return_type.as_deref()?;
    if !payload.regex_captures.is_empty()
        || payload.return_type_state!=ExpressionReturnTypeState::Static
        || payload.possible_return_types_state!=ExpressionPossibleReturnTypesState::Complete
        || payload.effective_possible_return_types_state!=ExpressionPossibleReturnTypesState::Complete
        || payload.effective_return_type.as_deref()!=Some(class)
        || payload.effective_possible_return_types!=payload.possible_return_types
        || payload.declared_multiplicity!=Some(DynamicMultiplicity::Single)
        || payload.effective_multiplicity!=payload.declared_multiplicity
        || !usable_bounds(class,&payload.possible_return_types) {return None;}
    Some(resolved_with_possible_types(class.into(),payload.possible_return_types.clone(),ExpressionPossibleReturnTypesState::Complete,
        DynamicMultiplicity::Single,Vec::new()))
}

fn usable_bounds(class:&str,bounds:&[String])->bool {
    !class.is_empty() && class.len()<=1024 && class!="java.lang.Object"
        && !bounds.is_empty() && bounds.len()<=64
        && bounds.iter().all(|c|!c.is_empty()&&c.len()<=1024&&c!="java.lang.Object")
        && bounds.iter().collect::<std::collections::BTreeSet<_>>().len()==bounds.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_complete_declaration_can_be_an_unknown_addon_class_without_an_inventory() {
        assert!(usable_bounds("unseen.Value",&["unseen.Value".into()]));
        assert!(usable_bounds("unseen.Base",&["unseen.Left".into(),"unseen.Right".into()]));
        for bounds in [Vec::new(),vec!["java.lang.Object".into()],vec!["unseen.Value".into(),"unseen.Value".into()]] {
            assert!(!usable_bounds("unseen.Value",&bounds));
        }
    }
}
