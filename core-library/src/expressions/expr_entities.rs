use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler, resolved_with_metadata,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprEntities";
const HANDLER_ID: &str = "core.expression.expr-entities";
const ENTITY: &str = "org.bukkit.entity.Entity";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        if payload.pattern_index > 5 {
            return SemanticResolution::Reject(
                "entities Expression has an unknown pattern index".to_owned(),
            );
        }
        let Some(entity_data) = payload.children.first() else {
            return SemanticResolution::Reject(
                "entities Expression requires an entity data Expression".to_owned(),
            );
        };
        let literal_pattern = payload.pattern_index.is_multiple_of(2);
        if literal_pattern {
            let plural = metadata_value(&entity_data.metadata, "entity-plural")
                .or_else(|| metadata_value(&entity_data.metadata, "literal-plural"));
            // ExprEntities.init() checks every value of the literal-only %*entitydatas% patterns.
            // Unknown plurality is accepted only when the complete expression explicitly starts
            // with "all"; dynamic odd-index patterns deliberately skip this validation.
            if plural == Some("false") || (plural.is_none() && !starts_with_all(&payload.input)) {
                return SemanticResolution::Reject(
                    "entities Expression requires plural entity data".to_owned(),
                );
            }
        }

        let represented_class = metadata_value(&entity_data.metadata, "entity-class")
            .or_else(|| metadata_value(&entity_data.metadata, "literal-represented-class"));
        let concrete_literal = represented_class.is_some()
            && entity_data.multiplicity == Some(DynamicMultiplicity::Single);
        let return_type = if concrete_literal {
            represented_class.expect("checked above")
        } else {
            ENTITY
        };
        resolved_with_metadata(
            return_type.to_owned(),
            DynamicMultiplicity::Multiple,
            vec![metadata(
                "semantic-mode",
                if concrete_literal {
                    "entities-literal-type"
                } else {
                    "entities-generic-type"
                },
            )],
        )
    })
}

fn starts_with_all(input: &str) -> bool {
    input
        .trim_start()
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("all"))
}

#[cfg(test)]
mod tests {
    use super::starts_with_all;

    #[test]
    fn detects_the_explicit_all_prefix_case_insensitively() {
        assert!(starts_with_all("all unknown entities"));
        assert!(starts_with_all("  ALL unknown entities"));
        assert!(!starts_with_all("players"));
    }
}
