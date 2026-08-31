use super::{SemanticResolution, matches, register_handler, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprSecCreateWorldBorder";
const HANDLER_ID: &str = "core.expression.expr-sec-create-world-border";
const WORLD_BORDER: &str = "org.bukkit.WorldBorder";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        resolved(
            WORLD_BORDER,
            DynamicMultiplicity::Single,
            "create-world-border",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{WORLD_BORDER, resolve};
    use crate::expressions::SemanticResolution;
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionPossibleReturnTypesState, ExpressionReturnTypeState, MappedSpan, ParseContext,
        RegisteredExpressionPayload, SourceOrigin, TextRange,
    };

    fn payload() -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            context: ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                values: Vec::new(),
            },
            input: "a virtual world border".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: "ch.njol.skript.sections.ExprSecCreateWorldBorder".to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "a [virtual] world[ ]border".to_owned(),
            span: MappedSpan {
                virtual_range: range,
                origins: vec![SourceOrigin {
                    original_range: range,
                    kind: crate::nlaocs::skript_parser_addon::types::OriginKind::Exact,
                    expansion: None,
                }],
            },
            expected_types: Vec::new(),
            declared_return_type: None,
            declared_multiplicity: None,
            return_type_state: ExpressionReturnTypeState::Dynamic,
            possible_return_types: Vec::new(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            time: 0,
            regex_captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            children: Vec::new(),
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: None,
            effective_possible_return_types: Vec::new(),
            effective_possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            effective_multiplicity: None,
            metadata: Vec::new(),
        }
    }

    #[test]
    fn section_expression_resolves_to_one_world_border() {
        assert!(matches!(
            resolve(&payload()),
            Some(SemanticResolution::Resolved {
                return_type,
                multiplicity:
                    crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity::Single,
                ..
            }) if return_type == WORLD_BORDER
        ));
    }
}
