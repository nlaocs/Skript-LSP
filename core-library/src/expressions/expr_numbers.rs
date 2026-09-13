use super::{SemanticResolution, matches, metadata, register_handler, resolved_with_metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprNumbers";
const HANDLER_ID: &str = "core.expression.expr-numbers";
const INTEGER: &str = "java.lang.Long";
const NUMBER: &str = "java.lang.Double";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_mark(payload.mark))
}

fn resolve_mark(mark: i32) -> SemanticResolution {
    let (return_type, numeric_kind) = match mark {
        0 => (NUMBER, "numbers"),
        1 => (INTEGER, "integers"),
        2 => (NUMBER, "decimals"),
        _ => {
            return SemanticResolution::Reject(format!(
                "numbers Expression has an unknown parse mark: {mark}"
            ));
        }
    };

    // ExprNumbers.getReturnType() depends on ParseResult.mark, while
    // isSingle() is always false for every mode.
    resolved_with_metadata(
        return_type.to_owned(),
        DynamicMultiplicity::Multiple,
        vec![
            metadata("semantic-mode", "number-range"),
            metadata("numeric-kind", numeric_kind),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{HANDLER_ID, INTEGER, NUMBER, resolve_mark};
    use crate::expressions::SemanticResolution;
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    #[test]
    fn numbers_and_decimals_return_double() {
        for mark in [0, 2] {
            let SemanticResolution::Resolved {
                return_type,
                possible_return_types,
                multiplicity,
                ..
            } = resolve_mark(mark)
            else {
                panic!("number range mark {mark} must resolve");
            };
            assert_eq!(return_type, NUMBER);
            assert_eq!(possible_return_types, [NUMBER]);
            assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
        }
    }

    #[test]
    fn integers_return_long_and_remain_multiple() {
        let SemanticResolution::Resolved {
            return_type,
            possible_return_types,
            multiplicity,
            ..
        } = resolve_mark(1)
        else {
            panic!("integer range mark must resolve");
        };
        assert_eq!(return_type, INTEGER);
        assert_eq!(possible_return_types, [INTEGER]);
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
    }

    #[test]
    fn unknown_parse_marks_are_rejected() {
        assert!(matches!(resolve_mark(3), SemanticResolution::Reject(_)));
    }

    #[test]
    fn handler_targets_expr_numbers() {
        let mut handlers = Vec::new();
        super::register(&mut handlers);
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].handler_id, HANDLER_ID);
    }
}
