use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::catalog::{self, DifferenceOption, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprDifference";
const HANDLER_ID: &str = "core.expression.expr-difference";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_difference(payload))
}

fn resolve_difference(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let [first, second] = payload.children.as_slice() else {
        return SemanticResolution::Reject(
            "difference Expression requires exactly two operands".to_owned(),
        );
    };
    let Some(first_type) = known_operand_type(first) else {
        return dynamic_difference();
    };
    let Some(second_type) = known_operand_type(second) else {
        return dynamic_difference();
    };

    let common =
        match catalog::common_assignable_class(&[first_type.to_owned(), second_type.to_owned()]) {
            Ok(Some(common)) => common,
            Ok(None) => {
                return unresolved("the common operand type is not present in the Catalog");
            }
            Err(reason) => {
                return unresolved(&format!("difference Catalog lookup failed: {reason}"));
            }
        };

    if common != OBJECT {
        return resolve_for_type(&common);
    }

    match difference_with_conversion(first_type, second_type) {
        Ok(Some(option)) => resolved_difference(option),
        Ok(None) => match difference_with_conversion(second_type, first_type) {
            Ok(Some(option)) => resolved_difference(option),
            Ok(None) => reject(first_type, second_type),
            Err(reason) => unresolved(&reason),
        },
        Err(reason) => unresolved(&reason),
    }
}

fn known_operand_type(
    child: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild,
) -> Option<&str> {
    if child.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete {
        return None;
    }
    child
        .return_type
        .as_deref()
        .filter(|return_type| !return_type.is_empty() && *return_type != OBJECT)
}

fn difference_with_conversion(
    target_type: &str,
    source_type: &str,
) -> Result<Option<DifferenceOption>, String> {
    let Some(option) = catalog::difference_options_for_type(target_type)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    match catalog::can_convert(source_type, target_type)? {
        TypeRelation::Compatible => Ok(Some(option)),
        TypeRelation::Incompatible => Ok(None),
        TypeRelation::Unknown => Err(format!(
            "whether {source_type} can be converted to {target_type} is unresolved"
        )),
    }
}

fn resolve_for_type(input_type: &str) -> SemanticResolution {
    match catalog::difference_options_for_type(input_type) {
        Ok(options) => options.into_iter().next().map_or_else(
            || {
                SemanticResolution::Reject(format!(
                    "there is no registered difference operation for {input_type}"
                ))
            },
            resolved_difference,
        ),
        Err(reason) => unresolved(&format!("difference Catalog lookup failed: {reason}")),
    }
}

fn resolved_difference(option: DifferenceOption) -> SemanticResolution {
    let return_type = option.return_class;
    resolved_with_possible_types(
        return_type.clone(),
        vec![return_type],
        ExpressionPossibleReturnTypesState::Complete,
        DynamicMultiplicity::Single,
        vec![
            metadata("semantic-mode", "difference"),
            metadata("difference-input-type", &option.input_class),
            metadata("difference-registration-id", &option.registration_id),
        ],
    )
}

fn dynamic_difference() -> SemanticResolution {
    unresolved("difference operand type is unresolved")
}

fn unresolved(reason: &str) -> SemanticResolution {
    SemanticResolution::Unresolved {
        reason: reason.to_owned(),
        metadata: vec![metadata("semantic-mode", "difference")],
    }
}

fn reject(first_type: &str, second_type: &str) -> SemanticResolution {
    SemanticResolution::Reject(format!(
        "cannot get the difference of {first_type} and {second_type}"
    ))
}

#[cfg(test)]
mod tests {
    use super::dynamic_difference;
    use crate::expressions::SemanticResolution;

    #[test]
    fn dynamic_operands_do_not_fake_an_object_return_type() {
        assert!(matches!(
            dynamic_difference(),
            SemanticResolution::Unresolved { .. }
        ));
    }
}
