use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, MetadataEntry,
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprElement";
const HANDLER_ID: &str = "core.expression.expr-element";
const SKRIPT_QUEUE: &str = "org.skriptlang.skript.lang.util.SkriptQueue";
const OBJECT: &str = "java.lang.Object";
const KEY_PROVIDER: &str = "expression.capability.key-provider";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(source) = payload.children.last() else {
            return SemanticResolution::Reject(
                "element Expression requires a source Expression".to_owned(),
            );
        };
        let Some(source_type) = source.return_type.as_deref() else {
            return SemanticResolution::Reject(
                "element Expression requires a typed source Expression".to_owned(),
            );
        };
        let generation = match crate::runtime::skript_at_least(2, 16) {
            Some(true) => ElementPatternGeneration::Modern,
            Some(false) => {
                if crate::runtime::skript_at_least(2, 10) == Some(true) {
                    ElementPatternGeneration::Queues
                } else {
                    ElementPatternGeneration::Legacy
                }
            }
            #[cfg(test)]
            None => infer_pattern_generation(payload.pattern_index, &payload.pattern),
            #[cfg(not(test))]
            None => {
                return SemanticResolution::Reject(
                    "element Expression requires the active Skript version".to_owned(),
                );
            }
        };
        let Some(multiple) = element_is_multiple(generation, payload.pattern_index) else {
            return SemanticResolution::Reject(
                "element Expression has an unknown pattern index".to_owned(),
            );
        };

        // ExprElement.getReturnType()/possibleReturnTypes() delegate to the source Expression.
        // SkriptQueue is the one exception: consuming a queue exposes Object values rather than
        // the queue object itself. The selected ElementType only controls isSingle().
        let (return_type, possible_return_types, possible_return_types_state) =
            if source_type == SKRIPT_QUEUE {
                (
                    OBJECT.to_owned(),
                    vec![OBJECT.to_owned()],
                    ExpressionPossibleReturnTypesState::Complete,
                )
            } else {
                let possible = if source.possible_return_types.is_empty() {
                    vec![source_type.to_owned()]
                } else {
                    source.possible_return_types.clone()
                };
                (
                    source_type.to_owned(),
                    possible,
                    source.possible_return_types_state,
                )
            };

        resolved_with_possible_types(
            return_type,
            possible_return_types,
            possible_return_types_state,
            if multiple {
                DynamicMultiplicity::Multiple
            } else {
                DynamicMultiplicity::Single
            },
            key_preserving_metadata("element-selection", &source.metadata),
        )
    })
}

fn key_preserving_metadata(mode: &str, source: &[MetadataEntry]) -> Vec<MetadataEntry> {
    let mut output = vec![metadata("semantic-mode", mode)];
    if metadata_value(source, KEY_PROVIDER) == Some("true") {
        output.push(metadata(KEY_PROVIDER, "true"));
    }
    output
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ElementPatternGeneration {
    Legacy,
    Queues,
    Modern,
}

#[cfg(test)]
fn infer_pattern_generation(pattern_index: u64, pattern: &str) -> ElementPatternGeneration {
    if pattern.contains("random [:distinct]") || pattern.contains("(of|in) %objects%") {
        ElementPatternGeneration::Modern
    } else if pattern_index > 4 || pattern.contains("%queue%") {
        ElementPatternGeneration::Queues
    } else {
        ElementPatternGeneration::Legacy
    }
}

fn element_is_multiple(generation: ElementPatternGeneration, pattern_index: u64) -> Option<bool> {
    // ExprElement has three registration layouts across supported Skript releases:
    // 2.6-2.9 has five object patterns, 2.10-2.15 appends five queue patterns,
    // and 2.16 folds queue handling into six object patterns and adds random-N.
    match generation {
        ElementPatternGeneration::Legacy => {
            (pattern_index <= 4).then_some(matches!(pattern_index, 1 | 4))
        }
        ElementPatternGeneration::Queues => {
            (pattern_index <= 9).then_some(matches!(pattern_index, 1 | 4 | 6 | 9))
        }
        ElementPatternGeneration::Modern => {
            (pattern_index <= 5).then_some(matches!(pattern_index, 1 | 3 | 5))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ElementPatternGeneration, KEY_PROVIDER, element_is_multiple, key_preserving_metadata,
    };
    use crate::nlaocs::skript_parser_addon::types::MetadataEntry;

    #[test]
    fn maps_pre_216_patterns_to_skript_multiplicity() {
        for pattern_index in [1, 4] {
            assert_eq!(
                element_is_multiple(ElementPatternGeneration::Legacy, pattern_index),
                Some(true)
            );
        }
        for pattern_index in [0, 2, 3] {
            assert_eq!(
                element_is_multiple(ElementPatternGeneration::Legacy, pattern_index),
                Some(false)
            );
        }
        assert_eq!(
            element_is_multiple(ElementPatternGeneration::Legacy, 5),
            None
        );
    }

    #[test]
    fn maps_210_through_215_queue_patterns_to_skript_multiplicity() {
        for pattern_index in [1, 4, 6, 9] {
            assert_eq!(
                element_is_multiple(ElementPatternGeneration::Queues, pattern_index),
                Some(true)
            );
        }
        for pattern_index in [0, 2, 3, 5, 7, 8] {
            assert_eq!(
                element_is_multiple(ElementPatternGeneration::Queues, pattern_index),
                Some(false)
            );
        }
        assert_eq!(
            element_is_multiple(ElementPatternGeneration::Queues, 10),
            None
        );
    }

    #[test]
    fn maps_216_patterns_to_skript_multiplicity() {
        for pattern_index in [1, 3, 5] {
            assert_eq!(
                element_is_multiple(ElementPatternGeneration::Modern, pattern_index),
                Some(true)
            );
        }
        for pattern_index in [0, 2, 4] {
            assert_eq!(
                element_is_multiple(ElementPatternGeneration::Modern, pattern_index),
                Some(false)
            );
        }
        assert_eq!(
            element_is_multiple(ElementPatternGeneration::Modern, 6),
            None
        );
    }

    #[test]
    fn preserves_keys_only_when_the_source_can_provide_them() {
        let keyed = [MetadataEntry {
            key: format!("source/{KEY_PROVIDER}"),
            value: "true".to_owned(),
            owner_component_id: None,
        }];
        assert!(
            key_preserving_metadata("element-selection", &keyed)
                .iter()
                .any(|entry| entry.key == KEY_PROVIDER && entry.value == "true")
        );
        assert!(
            !key_preserving_metadata("element-selection", &[])
                .iter()
                .any(|entry| entry.key == KEY_PROVIDER)
        );
    }
}
