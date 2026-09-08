use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, ParseResultStatus, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};

const SUPER_CLASS: &str =
    "org.skriptlang.skript.bukkit.potion.elements.effects.PotionPropertyEffect";
const HANDLER_ID: &str = "core.effect.potion-property";
const SKRIPT_DEFINITION_PREFIX: &str = "effect:skript:";
const POTION_EFFECT: &str =
    "org.skriptlang.skript.bukkit.potion.elements.expressions.ExprPotionEffect";
const POTION_EFFECTS: &str =
    "org.skriptlang.skript.bukkit.potion.elements.expressions.ExprPotionEffects";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Effect,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Effect,
        targets: vec![RegisteredSyntaxHandlerTarget::SuperClass(
            SUPER_CLASS.to_owned(),
        )],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    let is_skript_definition = payload
        .candidate
        .as_ref()
        .is_some_and(|candidate| is_skript_definition(&candidate.definition_id));
    if !is_skript_definition || !super::matches(&payload, HANDLER_ID) {
        return None;
    }

    let (property, pattern_index, pattern, negated) = {
        let candidate = payload.candidate.as_ref()?;
        (
            property_for_class(candidate.element_class.as_deref()),
            candidate.pattern_index,
            candidate.pattern.clone(),
            candidate.tags.iter().any(|tag| tag.value == "not"),
        )
    };

    let Some(property) = property else {
        return Some(unresolved(
            payload,
            "core.effect.potion-property.unknown-subclass",
            "this PotionPropertyEffect subclass is not known to CoreLibrary",
        ));
    };
    if !property.accepts_pattern(pattern_index, &pattern) {
        return Some(unresolved(
            payload,
            "core.effect.potion-property.unknown-pattern",
            "this potion property pattern is not known to CoreLibrary",
        ));
    }

    super::annotate(&mut payload, "semantic-mode", "potion-property");
    super::annotate(&mut payload, "potion-property", property.name());
    super::annotate(&mut payload, "potion-operation", property.operation());
    super::annotate(
        &mut payload,
        "potion-property-negated",
        if negated { "true" } else { "false" },
    );

    let Some(source) = successful_capture(&payload, 0) else {
        return Some(unresolved(
            payload,
            "core.effect.potion-property.unresolved-source",
            "the potion effect Expression could not be inspected",
        ));
    };
    let Some(summary) = source.summary.as_ref() else {
        return Some(unresolved(
            payload,
            "core.effect.potion-property.unresolved-source",
            "the potion effect Expression has no semantic summary",
        ));
    };
    let source_class = summary.element_class.as_deref();
    if source_class != Some(POTION_EFFECT) && source_class != Some(POTION_EFFECTS) {
        return Some(unresolved(
            payload,
            "core.effect.potion-property.unresolved-source",
            "the source Expression is not a known potion effect Expression",
        ));
    }

    let source_span = source.span.clone();
    let state = potion_state(summary);
    match state {
        PotionState::Active | PotionState::Unset => Some(super::accept(payload)),
        PotionState::Hidden | PotionState::Both => Some(super::reject_with(
            "hidden potion effects cannot be changed",
            "core.effect.potion-property.hidden-effects",
            source_span,
        )),
        PotionState::Unknown => Some(unresolved(
            payload,
            "core.effect.potion-property.unresolved-state",
            "the potion effect visibility state could not be resolved, so mutability was not guessed",
        )),
    }
}

fn is_skript_definition(definition_id: &str) -> bool {
    definition_id.starts_with(SKRIPT_DEFINITION_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PotionProperty {
    Ambient,
    Icon,
    Infinite,
    Particles,
}

impl PotionProperty {
    const fn name(self) -> &'static str {
        match self {
            Self::Ambient => "ambient",
            Self::Icon => "icon",
            Self::Infinite => "infinite",
            Self::Particles => "particles",
        }
    }

    const fn operation(self) -> &'static str {
        match self {
            Self::Ambient | Self::Infinite => "make",
            Self::Icon | Self::Particles => "show",
        }
    }

    fn accepts_pattern(self, pattern_index: u64, pattern: &str) -> bool {
        self.pattern(pattern_index)
            .is_some_and(|expected| expected == pattern.trim())
    }

    const fn pattern(self, pattern_index: u64) -> Option<&'static str> {
        match (self, pattern_index) {
            (Self::Ambient, 0) => Some("make %skriptpotioneffects% [:not] ambient"),
            (Self::Icon, 0) => {
                Some("(show|not:hide) [the] [potion] icon[s] [(of|for) %skriptpotioneffects%]")
            }
            (Self::Icon, 1) => Some("(show|not:hide) %skriptpotioneffects%'[s] icon"),
            (Self::Infinite, 0) => Some("make %skriptpotioneffects% [:not] (infinite|permanent)"),
            (Self::Particles, 0) => {
                Some("(show|not:hide) [the] [potion] particles [(of|for) %skriptpotioneffects%]")
            }
            (Self::Particles, 1) => Some("(show|not:hide) %skriptpotioneffects%'[s] particles"),
            _ => None,
        }
    }
}

fn property_for_class(class_name: Option<&str>) -> Option<PotionProperty> {
    match class_name {
        Some("org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionAmbient") => {
            Some(PotionProperty::Ambient)
        }
        Some("org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionIcon") => {
            Some(PotionProperty::Icon)
        }
        Some("org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionInfinite") => {
            Some(PotionProperty::Infinite)
        }
        Some("org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionParticles") => {
            Some(PotionProperty::Particles)
        }
        _ => None,
    }
}

fn successful_capture(
    payload: &EffectPayload,
    capture_index: u64,
) -> Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture> {
    super::parsed_capture(payload, capture_index)
        .filter(|capture| capture.status == ParseResultStatus::Success)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PotionState {
    Active,
    Hidden,
    Both,
    Unset,
    Unknown,
}

fn potion_state(summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary) -> PotionState {
    for key in ["potion-state", "semantic-potion-state"] {
        if let Some(value) = super::metadata_value(&summary.metadata, key) {
            return match value.trim().to_ascii_lowercase().as_str() {
                "active" => PotionState::Active,
                "hidden" => PotionState::Hidden,
                "both" => PotionState::Both,
                "unset" => PotionState::Unset,
                _ => PotionState::Unknown,
            };
        }
    }

    PotionState::Unknown
}

fn unresolved(mut payload: EffectPayload, code: &str, message: &str) -> HookOutput {
    let span = payload
        .candidate
        .as_ref()
        .map(|candidate| candidate.span.clone())
        .unwrap_or_else(|| payload.span.clone());
    super::mark_unresolved(&mut payload, code);
    super::continue_with_diagnostics(payload, vec![super::warning(code, message, span)])
}

#[cfg(test)]
mod tests {
    use super::{PotionProperty, property_for_class};

    #[test]
    fn all_native_potion_property_effects_are_mapped() {
        assert_eq!(
            property_for_class(Some(
                "org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionAmbient"
            )),
            Some(PotionProperty::Ambient)
        );
        assert_eq!(
            property_for_class(Some(
                "org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionIcon"
            )),
            Some(PotionProperty::Icon)
        );
        assert_eq!(
            property_for_class(Some(
                "org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionInfinite"
            )),
            Some(PotionProperty::Infinite)
        );
        assert_eq!(
            property_for_class(Some(
                "org.skriptlang.skript.bukkit.potion.elements.effects.EffPotionParticles"
            )),
            Some(PotionProperty::Particles)
        );
    }

    #[test]
    fn show_properties_have_two_native_patterns_but_make_has_one() {
        assert!(PotionProperty::Icon.accepts_pattern(
            0,
            "(show|not:hide) [the] [potion] icon[s] [(of|for) %skriptpotioneffects%]"
        ));
        assert!(
            PotionProperty::Icon
                .accepts_pattern(1, "(show|not:hide) %skriptpotioneffects%'[s] icon")
        );
        assert!(
            PotionProperty::Ambient.accepts_pattern(0, "make %skriptpotioneffects% [:not] ambient")
        );
        assert!(
            !PotionProperty::Ambient
                .accepts_pattern(1, "make %skriptpotioneffects% [:not] ambient")
        );
    }

    #[test]
    fn only_skript_definition_namespace_is_owned_by_this_handler() {
        assert!(super::is_skript_definition("effect:skript:abc"));
        assert!(!super::is_skript_definition("effect:skript-reflect:abc"));
        assert!(!super::is_skript_definition("effect:other-addon:abc"));
    }
}
