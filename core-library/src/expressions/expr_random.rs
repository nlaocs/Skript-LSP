use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::catalog::{self, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprRandom";
const HANDLER_ID: &str = "core.expression.expr-random";
const CLASS_INFO: &str = "ch.njol.skript.classes.ClassInfo";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(class_info) = payload
            .children
            .iter()
            .find(|child| child.return_type.as_deref() == Some(CLASS_INFO))
        else {
            return SemanticResolution::Reject(
                "random Expression requires a ClassInfo child".to_owned(),
            );
        };
        let Some(source_type) = payload.children.iter().find_map(|child| {
            (child.return_type.as_deref() != Some(CLASS_INFO))
                .then_some(child.return_type.as_deref())
                .flatten()
        }) else {
            return SemanticResolution::Reject(
                "random Expression requires a typed source Expression".to_owned(),
            );
        };

        let Some(selection_class) = metadata_value(&class_info.metadata, "target-class") else {
            return SemanticResolution::Reject(
                "random Expression ClassInfo has no represented Java class".to_owned(),
            );
        };
        let source = payload
            .children
            .iter()
            .find(|child| child.return_type.as_deref() != Some(CLASS_INFO))
            .expect("the typed source was found above");
        let mut converted_types = Vec::new();
        let mut unresolved = false;
        // `getConvertedExpression` builds converters from every possible return
        // type, including for ordinary Expressions. An unparsed ExpressionList
        // differs only in that Skript converts its members one by one.
        let candidate_types = if source.possible_return_types.is_empty() {
            vec![source_type]
        } else {
            source
                .possible_return_types
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        };
        for candidate in candidate_types {
            match catalog::is_class_assignable(candidate, selection_class) {
                Ok(TypeRelation::Compatible) => converted_types.push(candidate.to_owned()),
                Ok(TypeRelation::Incompatible) => {
                    match catalog::can_convert(candidate, selection_class) {
                        Ok(TypeRelation::Compatible) => {
                            converted_types.push(selection_class.to_owned())
                        }
                        Ok(TypeRelation::Unknown) | Err(_) => unresolved = true,
                        Ok(TypeRelation::Incompatible) => {}
                    }
                }
                Ok(TypeRelation::Unknown) | Err(_) => {
                    match catalog::can_convert(candidate, selection_class) {
                        Ok(TypeRelation::Compatible) => {
                            converted_types.push(selection_class.to_owned())
                        }
                        Ok(TypeRelation::Unknown) | Err(_) => unresolved = true,
                        Ok(TypeRelation::Incompatible) => {}
                    }
                }
            }
        }
        converted_types.sort();
        converted_types.dedup();
        if converted_types.is_empty() && !unresolved {
            return SemanticResolution::Reject(format!(
                "source Expression cannot be converted to {selection_class}"
            ));
        }
        if converted_types.is_empty() {
            converted_types.push(selection_class.to_owned());
        }
        let return_type = if converted_types.len() == 1 {
            converted_types[0].clone()
        } else {
            match catalog::common_assignable_class(&converted_types) {
                Ok(Some(return_type)) => return_type,
                Ok(None) | Err(_) => {
                    unresolved = true;
                    selection_class.to_owned()
                }
            }
        };
        let mut output_metadata = vec![
            metadata("semantic-mode", "random-element"),
            metadata("selection-class", selection_class),
        ];
        if unresolved {
            output_metadata.push(metadata("conversion-state", "unresolved"));
        }
        resolved_with_possible_types(
            return_type,
            converted_types,
            if unresolved
                || source.possible_return_types_state
                    != ExpressionPossibleReturnTypesState::Complete
            {
                ExpressionPossibleReturnTypesState::Partial
            } else {
                ExpressionPossibleReturnTypesState::Complete
            },
            DynamicMultiplicity::Single,
            output_metadata,
        )
    })
}
