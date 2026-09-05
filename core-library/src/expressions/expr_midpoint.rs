use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::{
    catalog::{self, TypeRelation},
    nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
        RegisteredExpressionPayload, RegisteredSyntaxHandler,
    },
};

const CLASS_SUFFIX: &str = ".ExprMidpoint";
const HANDLER_ID: &str = "core.expression.expr-midpoint";
const LOCATION: &str = "org.bukkit.Location";
const VECTOR: &str = "org.bukkit.util.Vector";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_midpoint(payload))
}

fn resolve_midpoint(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let [first, second] = payload.children.as_slice() else {
        return SemanticResolution::Reject(
            "midpoint Expression requires two source Expressions".to_owned(),
        );
    };

    let first_types = match midpoint_operand_types(first) {
        Ok(types) => types,
        Err(reason) => {
            return unresolved(format!(
                "midpoint first operand type relation is unresolved: {reason}"
            ));
        }
    };
    let second_types = match midpoint_operand_types(second) {
        Ok(types) => types,
        Err(reason) => {
            return unresolved(format!(
                "midpoint second operand type relation is unresolved: {reason}"
            ));
        }
    };
    if first_types.len() == 1 && second_types.len() == 1 && first_types[0] != second_types[0] {
        return SemanticResolution::Reject(
            "the midpoint requires two locations or two vectors of the same kind".to_owned(),
        );
    }

    // ExprMidpoint keeps the longer checkExpressionType result as possibleReturnTypes().
    // That preserves [Location, Vector] when one operand is still type-ambiguous.
    let possible_return_types = if first_types.len() > second_types.len() {
        first_types
    } else {
        second_types
    };
    let return_type = match midpoint_return_type(&possible_return_types) {
        Ok(return_type) => return_type,
        Err(reason) => return unresolved(reason),
    };
    let possible_return_types_state = merge_possible_type_states(
        first.possible_return_types_state,
        second.possible_return_types_state,
    );

    resolved_with_possible_types(
        return_type,
        possible_return_types,
        possible_return_types_state,
        DynamicMultiplicity::Single,
        vec![metadata("semantic-mode", "midpoint")],
    )
}

fn midpoint_operand_types(child: &RegisteredExpressionChild) -> Result<Vec<String>, String> {
    let possible = if child.possible_return_types.is_empty() {
        child.return_type.iter().cloned().collect::<Vec<_>>()
    } else {
        child.possible_return_types.clone()
    };
    if possible.is_empty() {
        return Ok(vec![LOCATION.to_owned(), VECTOR.to_owned()]);
    }
    let can_location = can_return_as(&possible, LOCATION)?;
    let can_vector = can_return_as(&possible, VECTOR)?;
    if can_location && !can_vector {
        Ok(vec![LOCATION.to_owned()])
    } else if can_vector && !can_location {
        Ok(vec![VECTOR.to_owned()])
    } else {
        // This is the same conservative result as Java's checkExpressionType():
        // an unknown or mixed expression may still be either supported class.
        Ok(vec![LOCATION.to_owned(), VECTOR.to_owned()])
    }
}

fn can_return_as(possible: &[String], target: &str) -> Result<bool, String> {
    let mut unresolved_reason = None;
    for class_name in possible {
        if class_name == OBJECT {
            // Expression.canReturn treats Object as a wildcard.
            return Ok(true);
        }
        let relation = if class_name == target {
            TypeRelation::Compatible
        } else if is_supported_kind(class_name) && is_supported_kind(target) {
            // The non-WASM catalog intentionally only knows exact classes; these two
            // registered Skript types are nevertheless distinct native targets.
            TypeRelation::Incompatible
        } else {
            match catalog::is_class_assignable(class_name, target) {
                Ok(relation) => relation,
                Err(reason) => {
                    unresolved_reason.get_or_insert(format!(
                        "type relation from {class_name} to {target} is unavailable: {reason}"
                    ));
                    continue;
                }
            }
        };
        match relation {
            TypeRelation::Compatible => return Ok(true),
            TypeRelation::Incompatible => {}
            TypeRelation::Unknown => {
                unresolved_reason.get_or_insert(format!(
                    "type relation from {class_name} to {target} is unknown"
                ));
            }
        };
    }
    match unresolved_reason {
        Some(reason) => Err(reason),
        None => Ok(false),
    }
}

fn is_supported_kind(class_name: &str) -> bool {
    class_name == LOCATION || class_name == VECTOR
}

fn midpoint_return_type(possible: &[String]) -> Result<String, String> {
    match possible {
        [] => Err("midpoint has no possible return type".to_owned()),
        [only] => Ok(only.clone()),
        _ => catalog::common_assignable_class(possible)
            .map_err(|reason| format!("midpoint common type is unavailable: {reason}"))?
            .ok_or_else(|| "midpoint has no common assignable return type".to_owned()),
    }
}

fn unresolved(reason: impl Into<String>) -> SemanticResolution {
    SemanticResolution::Unresolved {
        reason: reason.into(),
        metadata: vec![metadata("semantic-mode", "midpoint")],
    }
}

fn merge_possible_type_states(
    first: ExpressionPossibleReturnTypesState,
    second: ExpressionPossibleReturnTypesState,
) -> ExpressionPossibleReturnTypesState {
    use ExpressionPossibleReturnTypesState::{Complete, Partial, Unresolved};
    if first == Unresolved || second == Unresolved {
        Unresolved
    } else if first == Partial || second == Partial {
        Partial
    } else {
        Complete
    }
}

#[cfg(test)]
mod tests {
    use super::{LOCATION, VECTOR, midpoint_operand_types, midpoint_return_type};
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
    };

    fn child(types: &[&str], return_type: Option<&str>) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            text: "value".to_owned(),
            kind: "expression".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: return_type.map(str::to_owned),
            possible_return_types: types.iter().map(|value| (*value).to_owned()).collect(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: None,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn recognizes_the_two_supported_operand_kinds() {
        assert_eq!(
            midpoint_operand_types(&child(&[LOCATION], None)).expect("location is known"),
            [LOCATION]
        );
        assert_eq!(
            midpoint_operand_types(&child(&[VECTOR], None)).expect("vector is known"),
            [VECTOR]
        );
    }

    #[test]
    fn unknown_or_mixed_operands_keep_both_possibilities() {
        assert_eq!(
            midpoint_operand_types(&child(&[], Some("java.lang.Object")))
                .expect("Object is a wildcard"),
            [LOCATION, VECTOR]
        );
    }

    #[test]
    fn ambiguous_operands_have_object_as_the_conservative_common_type() {
        assert_eq!(
            midpoint_return_type(&[LOCATION.to_owned(), VECTOR.to_owned()])
                .expect("the catalog has a conservative common type"),
            "java.lang.Object"
        );
    }

    #[test]
    fn unknown_operand_assignability_is_unresolved() {
        assert!(midpoint_operand_types(&child(&["test.Unknown"], None)).is_err());
    }
}
