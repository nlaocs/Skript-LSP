use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffSuppressTypeHints";
const HANDLER_ID: &str = "core.effect.eff-suppress-type-hints";
const TYPE_HINTS_ACTIVE_KEY: &str = "parser.type-hints-active";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate = payload.candidate.as_ref()?;
    let active = type_hints_active(candidate);
    let syntax_context = payload.context.syntax_context;
    super::annotate(&mut payload, "semantic-mode", "suppress-type-hints");
    super::annotate(
        &mut payload,
        "type-hints-active",
        if active { "true" } else { "false" },
    );
    let mut output = super::accept(payload);
    super::add_context_update(
        &mut output,
        syntax_context,
        TYPE_HINTS_ACTIVE_KEY,
        Some(if active { b"true" } else { b"false" }),
    );
    Some(output)
}

fn type_hints_active(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
) -> bool {
    // EffSuppressTypeHints.init uses ParseResult.hasTag("stop"): the first
    // pattern's `un` tag intentionally starts suppression, while the second
    // pattern's `stop` tag re-enables hints.
    candidate.tags.iter().any(|tag| tag.value == "stop")
}

#[cfg(test)]
mod tests {
    use super::type_hints_active;
    use crate::nlaocs::skript_parser_addon::types::{
        EffectCandidate, EffectTag, MappedSpan, TextRange,
    };

    fn candidate(tags: &[&str]) -> EffectCandidate {
        EffectCandidate {
            raw_node_id: 0,
            definition_id: String::new(),
            registration_id: String::new(),
            element_class: None,
            priority: 0,
            registration_order: 0,
            pattern_index: 0,
            pattern: String::new(),
            span: MappedSpan {
                virtual_range: TextRange { start: 0, end: 0 },
                origins: Vec::new(),
            },
            captures: Vec::new(),
            tags: tags
                .iter()
                .map(|value| EffectTag {
                    value: (*value).to_owned(),
                    pattern_span: TextRange { start: 0, end: 0 },
                    input_span: MappedSpan {
                        virtual_range: TextRange { start: 0, end: 0 },
                        origins: Vec::new(),
                    },
                    implicit: false,
                })
                .collect(),
            mark: 0,
            marks: Vec::new(),
            handler: None,
            metadata: Vec::new(),
            parsed_captures: Vec::new(),
        }
    }

    #[test]
    fn start_disables_and_stop_reenables_type_hints() {
        assert!(!type_hints_active(&candidate(&[])));
        assert!(type_hints_active(&candidate(&["stop"])));
        assert!(!type_hints_active(&candidate(&["un"])));
    }
}
