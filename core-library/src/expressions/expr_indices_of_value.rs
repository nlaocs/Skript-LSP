use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprIndicesOfValue";
const HANDLER_ID: &str = "core.expression.expr-indices-of-value";
const KEY_PROVIDER: &str = "expression.capability.key-provider";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_indices(payload))
}

fn resolve_indices(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let [needle, haystack] = payload.children.as_slice() else {
        return SemanticResolution::Reject(
            "indices of value requires a needle and haystack Expression".to_owned(),
        );
    };
    if payload.pattern_index > 0 {
        match haystack.multiplicity {
            Some(DynamicMultiplicity::Single) => {
                return SemanticResolution::Reject(
                    "indices or positions in a list require a multiple-valued haystack".to_owned(),
                );
            }
            Some(DynamicMultiplicity::Multiple | DynamicMultiplicity::Both) => {}
            None => {
                return SemanticResolution::Unresolved {
                    reason: "whether the list haystack is multiple-valued is unresolved".to_owned(),
                    metadata: vec![metadata("semantic-mode", "value-positions")],
                };
            }
        }
    }
    if payload.pattern_index == 2
        && metadata_value(&haystack.metadata, KEY_PROVIDER) != Some("true")
    {
        return SemanticResolution::Unresolved {
            reason: "whether the haystack can provide keys is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "value-indices")],
        };
    }

    let all = payload.mark == 3
        || payload.mark == 0
            && payload
                .tags
                .iter()
                .any(|tag| tag.value.eq_ignore_ascii_case("mult"));
    let Some(multiplicity) = selection_multiplicity(
        all,
        needle.multiplicity,
        crate::runtime::skript_at_least(2, 14),
    ) else {
        return SemanticResolution::Unresolved {
            reason: "indices of value selection multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "value-selection")],
        };
    };
    let (return_type, mode) = if payload.pattern_index == 2 {
        ("java.lang.String", "value-indices")
    } else {
        ("java.lang.Long", "value-positions")
    };
    resolved_with_possible_types(
        return_type.to_owned(),
        vec![return_type.to_owned()],
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        vec![
            metadata("semantic-mode", mode),
            metadata("selection", selection(payload.mark, all)),
        ],
    )
}

fn selection_multiplicity(
    all: bool,
    needle: Option<DynamicMultiplicity>,
    multiple_needle_supported: Option<bool>,
) -> Option<DynamicMultiplicity> {
    if all {
        return Some(DynamicMultiplicity::Multiple);
    }

    match multiple_needle_supported {
        // Up to 2.13, the first/last forms always selected one value. Skript
        // 2.14 added support for selecting from multiple needles.
        Some(false) => Some(DynamicMultiplicity::Single),
        Some(true) => needle,
        None => match needle {
            // This is the only value whose result is identical before and
            // after the 2.14 behavior change.
            Some(DynamicMultiplicity::Single) => Some(DynamicMultiplicity::Single),
            Some(DynamicMultiplicity::Multiple | DynamicMultiplicity::Both) | None => None,
        },
    }
}

fn selection(mark: i32, all: bool) -> &'static str {
    if all {
        "all"
    } else if mark == 2 {
        "last"
    } else {
        "first"
    }
}

#[cfg(test)]
mod tests {
    use super::{selection, selection_multiplicity};
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    #[test]
    fn default_mult_tag_and_explicit_marks_select_the_native_mode() {
        assert_eq!(selection(0, false), "first");
        assert_eq!(selection(2, false), "last");
        assert_eq!(selection(0, true), "all");
        assert_eq!(selection(3, true), "all");
    }

    #[test]
    fn all_selection_is_multiple_even_without_needle_metadata() {
        assert_eq!(
            selection_multiplicity(true, None, None),
            Some(DynamicMultiplicity::Multiple)
        );
    }

    #[test]
    fn pre_214_first_and_last_are_always_single() {
        assert_eq!(
            selection_multiplicity(false, Some(DynamicMultiplicity::Multiple), Some(false)),
            Some(DynamicMultiplicity::Single)
        );
    }

    #[test]
    fn post_214_versions_delegate_first_and_last_to_the_needle() {
        assert_eq!(
            selection_multiplicity(false, Some(DynamicMultiplicity::Multiple), Some(true)),
            Some(DynamicMultiplicity::Multiple)
        );
        assert_eq!(
            selection_multiplicity(false, Some(DynamicMultiplicity::Both), Some(true)),
            Some(DynamicMultiplicity::Both)
        );
    }

    #[test]
    fn unknown_version_only_resolves_when_both_eras_have_the_same_result() {
        assert_eq!(
            selection_multiplicity(false, Some(DynamicMultiplicity::Single), None),
            Some(DynamicMultiplicity::Single)
        );
        assert_eq!(
            selection_multiplicity(false, Some(DynamicMultiplicity::Multiple), None),
            None
        );
    }
}
