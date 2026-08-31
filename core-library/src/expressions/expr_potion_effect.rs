use std::collections::BTreeMap;

use super::{
    SemanticResolution, matches, metadata, register_handler_targets, resolved_with_possible_types,
};
use crate::catalog::{AcceptedChangeType, ChangeContract, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.expression.expr-potion-effect";
const POTION_EFFECT: &str =
    "org.skriptlang.skript.bukkit.potion.elements.expressions.ExprPotionEffect";
const POTION_EFFECTS: &str =
    "org.skriptlang.skript.bukkit.potion.elements.expressions.ExprPotionEffects";
const ENTITY: &str = "org.bukkit.entity.Entity";
const OBJECT: &str = "java.lang.Object";
const SKRIPT_POTION_EFFECT: &str = "org.skriptlang.skript.bukkit.potion.util.SkriptPotionEffect";
const BUKKIT_POTION_EFFECT: &str = "org.bukkit.potion.PotionEffect";
const POTION_EFFECT_TYPE: &str = "org.bukkit.potion.PotionEffectType";
const TIMESPAN: &str = "ch.njol.skript.util.Timespan";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    // Full class names keep this 2.14+ handler away from the unrelated legacy
    // classes that used the same simple names.
    register_handler_targets(handlers, HANDLER_ID, &[POTION_EFFECT, POTION_EFFECTS]);
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_potion_effect(payload))
}

fn resolve_potion_effect(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let state = PotionState::from_tags(payload);
    let source_index = match source_child_index(&payload.element_class, payload.pattern_index) {
        Some(index) => index,
        _ => {
            return SemanticResolution::Reject("unknown potion effect Expression class".to_owned());
        }
    };
    let Some(source) = payload.children.get(source_index) else {
        return SemanticResolution::Unresolved {
            reason: "potion effect source Expression is unavailable".to_owned(),
            metadata: state.metadata(),
        };
    };
    if state.includes_hidden() {
        match can_return(source, ENTITY) {
            Ok(TypeRelation::Compatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => {
                return SemanticResolution::Unresolved {
                    reason:
                        "whether the hidden potion effect source can return an entity is unresolved"
                            .to_owned(),
                    metadata: state.metadata(),
                };
            }
            Ok(TypeRelation::Incompatible) => {
                return SemanticResolution::Reject(
                    "hidden potion effects require an entity source".to_owned(),
                );
            }
        }
    }

    let type_multiplicity = payload
        .children
        .get((payload.pattern_index % 2) as usize)
        .and_then(|types| types.multiplicity);
    let Some(multiplicity) = potion_multiplicity(
        payload.element_class.as_str(),
        state.includes_hidden(),
        type_multiplicity,
    ) else {
        return SemanticResolution::Unresolved {
            reason: "potion effect type multiplicity is unresolved".to_owned(),
            metadata: state.metadata(),
        };
    };
    let mut output_metadata = state.metadata();
    output_metadata.push(metadata("semantic-mode", "potion-effect"));
    let contract = change_contract(payload.element_class.as_str(), state);
    if let Ok(contract) =
        crate::catalog::change_contract_metadata(&payload.registration_id, &contract)
    {
        output_metadata.push(contract);
    }
    resolved_with_possible_types(
        SKRIPT_POTION_EFFECT.to_owned(),
        vec![SKRIPT_POTION_EFFECT.to_owned()],
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        output_metadata,
    )
}

fn potion_multiplicity(
    element_class: &str,
    includes_hidden: bool,
    type_multiplicity: Option<DynamicMultiplicity>,
) -> Option<DynamicMultiplicity> {
    if element_class == POTION_EFFECTS || includes_hidden {
        // ExprPotionEffects and hidden-effect selections enumerate effects;
        // the official isSingle() implementation returns false in both cases.
        Some(DynamicMultiplicity::Multiple)
    } else {
        // ExprPotionEffect delegates isSingle() to its type expression when
        // hidden effects are not requested. An explicit Both is therefore a
        // meaningful result, while missing child metadata is not.
        type_multiplicity
    }
}

fn source_child_index(class_name: &str, pattern_index: u64) -> Option<usize> {
    match class_name {
        POTION_EFFECT => Some(pattern_index.is_multiple_of(2) as usize),
        POTION_EFFECTS => Some(0),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PotionState {
    Unset,
    Active,
    Hidden,
    Both,
}

impl PotionState {
    fn from_tags(payload: &RegisteredExpressionPayload) -> Self {
        payload
            .tags
            .iter()
            .find_map(|tag| match tag.value.as_str() {
                "active" => Some(Self::Active),
                "hidden" => Some(Self::Hidden),
                "both" => Some(Self::Both),
                _ => None,
            })
            .unwrap_or(Self::Unset)
    }

    const fn includes_hidden(self) -> bool {
        matches!(self, Self::Hidden | Self::Both)
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Active => "active",
            Self::Hidden => "hidden",
            Self::Both => "both",
        }
    }

    fn metadata(self) -> Vec<crate::nlaocs::skript_parser_addon::types::MetadataEntry> {
        vec![metadata("potion-state", self.as_str())]
    }
}

fn can_return(source: &RegisteredExpressionChild, expected: &str) -> Result<TypeRelation, String> {
    let types = if source.possible_return_types.is_empty() {
        source.return_type.iter().collect::<Vec<_>>()
    } else {
        source.possible_return_types.iter().collect::<Vec<_>>()
    };
    let mut unknown = types.is_empty();
    for source_type in types {
        if source_type == OBJECT {
            return Ok(TypeRelation::Compatible);
        }
        match crate::catalog::is_class_assignable(source_type, expected)? {
            TypeRelation::Compatible => return Ok(TypeRelation::Compatible),
            TypeRelation::Incompatible => {}
            TypeRelation::Unknown => unknown = true,
        }
    }
    Ok(if unknown {
        TypeRelation::Unknown
    } else {
        TypeRelation::Incompatible
    })
}

fn change_contract(class_name: &str, state: PotionState) -> ChangeContract {
    let mut modes = BTreeMap::new();
    if class_name == POTION_EFFECTS {
        if !state.includes_hidden() {
            modes.insert("ADD".to_owned(), vec![accepted(BUKKIT_POTION_EFFECT, true)]);
            modes.insert("SET".to_owned(), vec![accepted(BUKKIT_POTION_EFFECT, true)]);
        }
        modes.insert(
            "REMOVE".to_owned(),
            vec![accepted(SKRIPT_POTION_EFFECT, true)],
        );
        for mode in ["DELETE", "RESET", "REMOVE_ALL"] {
            modes.insert(mode.to_owned(), vec![accepted(POTION_EFFECT_TYPE, true)]);
        }
    } else {
        for mode in ["ADD", "DELETE", "RESET"] {
            modes.insert(mode.to_owned(), vec![accepted(TIMESPAN, false)]);
        }
        let mut remove = vec![accepted(TIMESPAN, false)];
        if state.includes_hidden() {
            remove.insert(0, accepted(SKRIPT_POTION_EFFECT, true));
        }
        modes.insert("REMOVE".to_owned(), remove);
    }
    ChangeContract::Resolved { modes }
}

fn accepted(class_name: &str, multiple: bool) -> AcceptedChangeType {
    AcceptedChangeType {
        class_name: class_name.to_owned(),
        multiple,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_single_effects_become_multiple_and_accept_hidden_removal() {
        let ChangeContract::Resolved { modes } =
            change_contract(POTION_EFFECT, PotionState::Hidden)
        else {
            unreachable!()
        };
        assert_eq!(modes["REMOVE"][0], accepted(SKRIPT_POTION_EFFECT, true));
    }

    #[test]
    fn hidden_effect_collections_reject_set_and_add() {
        let ChangeContract::Resolved { modes } =
            change_contract(POTION_EFFECTS, PotionState::Hidden)
        else {
            unreachable!()
        };
        assert!(!modes.contains_key("SET"));
        assert!(!modes.contains_key("ADD"));
    }

    #[test]
    fn property_pattern_order_only_moves_the_specific_effect_source() {
        assert_eq!(source_child_index(POTION_EFFECT, 0), Some(1));
        assert_eq!(source_child_index(POTION_EFFECT, 1), Some(0));
        assert_eq!(source_child_index(POTION_EFFECTS, 0), Some(0));
        assert_eq!(source_child_index(POTION_EFFECTS, 1), Some(0));
    }

    #[test]
    fn collection_and_hidden_effects_are_multiple_without_type_metadata() {
        assert_eq!(
            potion_multiplicity(POTION_EFFECTS, false, None),
            Some(DynamicMultiplicity::Multiple)
        );
        assert_eq!(
            potion_multiplicity(POTION_EFFECT, true, None),
            Some(DynamicMultiplicity::Multiple)
        );
    }

    #[test]
    fn singular_effects_delegate_or_remain_unresolved() {
        assert_eq!(potion_multiplicity(POTION_EFFECT, false, None), None);
        assert_eq!(
            potion_multiplicity(POTION_EFFECT, false, Some(DynamicMultiplicity::Both)),
            Some(DynamicMultiplicity::Both)
        );
    }
}
