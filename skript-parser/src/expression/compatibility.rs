//! Shared native type rules and conservative editing-time mismatch evidence.
use super::ExpressionExpectedType;
use syntaxes::{Catalog, ClassName, Multiplicity};

/// True when native group/list/arithmetic parsing can contribute a competing
/// expression path. Editor pattern-only proofs must then retain Unknown.
/// This shares the native split/parenthesis/operator recognizers, not a second
/// spelling list or precedence implementation.
pub fn expression_has_intrinsic_shape(catalog: &Catalog, input: &str) -> bool {
    let range = crate::pattern_match::java_trim_range(input);
    let grouped = input
        .get(range.start..range.end)
        .is_some_and(|s| s.starts_with('('))
        && crate::pattern_match::find_parenthesis_end(input, range.start + 1, range.end)
            .is_some_and(|close| close + 1 == range.end);
    grouped
        || crate::expression_list::split_expression_list(input, range).is_some()
        || crate::arithmetic::has_operator_shape(catalog, input, range)
}

/// Native parser acceptance for one effective return class.
/// Generic Object must first be narrowed by a semantic provider.
pub fn expression_return_type_matches(
    catalog: &Catalog,
    return_type: Option<&ClassName>,
    expected_types: &[ExpressionExpectedType],
) -> bool {
    expected_types.is_empty()
        || return_type.is_some_and(|return_type| {
            expected_types.iter().any(|expected| {
                let from = return_type.as_str();
                let to = expected.class_name.as_str();
                to == "java.lang.Object"
                    || (from != "java.lang.Object" && catalog.can_convert(from, to))
            })
        })
}

/// Native parser acceptance for expression cardinality, independent of class.
pub fn expression_multiplicity_matches(
    multiplicity: Option<Multiplicity>,
    expected_types: &[ExpressionExpectedType],
) -> bool {
    expected_types.is_empty()
        || expected_types.iter().any(|expected| match multiplicity {
            None | Some(Multiplicity::Both | Multiplicity::Single) => true,
            Some(Multiplicity::Multiple) => expected.plural,
        })
}

/// Whether current, complete type information proves an editing candidate incompatible.
///
/// `complete` must be false when a provider can still replace the return types.
/// Unknown classes, runtime Object and absent alternatives never prove a mismatch.
/// Unknown cardinality can still prove a class mismatch, but cannot prove a
/// cardinality mismatch. The same native union and converter rules are used.
pub fn expression_types_are_incompatible(
    catalog: &Catalog,
    actual: &[ClassName],
    complete: bool,
    multiplicity: Option<Multiplicity>,
    expected: &[ExpressionExpectedType],
) -> bool {
    if !complete
        || actual.is_empty()
        || expected.is_empty()
        || actual.iter().any(|class| {
            class.as_str() == "java.lang.Object" || catalog.class(class.as_str()).is_none()
        })
        || expected
            .iter()
            .any(|ty| catalog.class(ty.class_name.as_str()).is_none())
    {
        return false;
    }
    !actual
        .iter()
        .any(|class| expression_return_type_matches(catalog, Some(class), expected))
        || !expression_multiplicity_matches(multiplicity, expected)
}
