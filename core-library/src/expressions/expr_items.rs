use super::{SemanticResolution, matches, metadata, register_handler, resolved_with_metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprItems";
const HANDLER_ID: &str = "core.expression.expr-items";
const ITEM_STACK: &str = "org.bukkit.inventory.ItemStack";
const ITEM_TYPE: &str = "ch.njol.skript.aliases.ItemType";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(generation) = generation(payload) else {
            return SemanticResolution::Reject(
                "items Expression requires a known Skript registration generation".to_owned(),
            );
        };
        let Some(pattern) = pattern_semantics(generation, payload.pattern_index) else {
            return SemanticResolution::Reject(
                "items Expression has an unknown pattern index for this Skript version".to_owned(),
            );
        };

        // ExprItems is deliberately plural in every upstream generation, even when
        // its input is a single ItemType expression. The old implementation yielded
        // ItemStacks; the 2.7.0 rewrite yields ItemTypes instead.
        resolved_with_metadata(
            pattern.return_type.to_owned(),
            DynamicMultiplicity::Multiple,
            vec![
                metadata("semantic-mode", "items"),
                metadata("items-kind", pattern.kind),
                metadata(
                    "items-registration-generation",
                    generation.as_metadata_value(),
                ),
            ],
        )
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegistrationGeneration {
    Legacy,
    ItemType,
}

impl RegistrationGeneration {
    fn as_metadata_value(self) -> &'static str {
        match self {
            Self::Legacy => "legacy-item-stack",
            Self::ItemType => "item-type",
        }
    }
}

fn generation(payload: &RegisteredExpressionPayload) -> Option<RegistrationGeneration> {
    if let Some(modern) = crate::runtime::skript_at_least(2, 7) {
        return Some(if modern {
            RegistrationGeneration::ItemType
        } else {
            RegistrationGeneration::Legacy
        });
    }

    // Unit tests and hosts that have not initialized the runtime profile can
    // still identify the exact upstream registration from its declared result.
    match payload.declared_return_type.as_deref() {
        Some(ITEM_STACK) => Some(RegistrationGeneration::Legacy),
        Some(ITEM_TYPE) => Some(RegistrationGeneration::ItemType),
        _ if payload.pattern.contains("item(s|[ ]types)")
            || payload.pattern.contains("block(s|[ ]types)") =>
        {
            Some(RegistrationGeneration::Legacy)
        }
        _ if payload.pattern.contains("block[[ ]type]s")
            || payload.pattern.contains("every block[[ ]type]")
            || payload.pattern.contains("block[s] of type[s]")
            || payload.pattern.contains("item[s] of type[s]") =>
        {
            Some(RegistrationGeneration::ItemType)
        }
        _ => None,
    }
}

struct PatternSemantics {
    return_type: &'static str,
    kind: &'static str,
}

fn pattern_semantics(
    generation: RegistrationGeneration,
    pattern_index: u64,
) -> Option<PatternSemantics> {
    match generation {
        RegistrationGeneration::Legacy => match pattern_index {
            0 | 1 => Some(PatternSemantics {
                return_type: ITEM_STACK,
                kind: "items",
            }),
            2 | 3 => Some(PatternSemantics {
                return_type: ITEM_STACK,
                kind: "blocks",
            }),
            _ => None,
        },
        RegistrationGeneration::ItemType => match pattern_index {
            0..=2 => Some(PatternSemantics {
                return_type: ITEM_TYPE,
                kind: "blocks",
            }),
            3 => Some(PatternSemantics {
                return_type: ITEM_TYPE,
                kind: "items",
            }),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{ITEM_STACK, ITEM_TYPE, RegistrationGeneration, pattern_semantics};

    #[test]
    fn legacy_patterns_return_item_stacks_and_keep_block_split() {
        assert_eq!(
            pattern_semantics(RegistrationGeneration::Legacy, 0)
                .expect("legacy item pattern")
                .return_type,
            ITEM_STACK
        );
        assert_eq!(
            pattern_semantics(RegistrationGeneration::Legacy, 2)
                .expect("legacy block pattern")
                .kind,
            "blocks"
        );
        assert!(pattern_semantics(RegistrationGeneration::Legacy, 4).is_none());
    }

    #[test]
    fn item_type_patterns_return_item_types_and_put_items_at_index_three() {
        for index in [0, 1, 2] {
            let semantics = pattern_semantics(RegistrationGeneration::ItemType, index)
                .expect("modern block pattern");
            assert_eq!(semantics.return_type, ITEM_TYPE);
            assert_eq!(semantics.kind, "blocks");
        }
        assert_eq!(
            pattern_semantics(RegistrationGeneration::ItemType, 3)
                .expect("modern item pattern")
                .kind,
            "items"
        );
        assert!(pattern_semantics(RegistrationGeneration::ItemType, 4).is_none());
    }
}
