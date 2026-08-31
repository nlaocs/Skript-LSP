use std::collections::BTreeMap;

use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::{
    catalog::{self, AcceptedChangeType, ChangeContract},
    nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
        RegisteredSyntaxHandler,
    },
};

const CLASS_SUFFIX: &str = ".ExprCustomModelData";
const HANDLER_ID: &str = "core.expression.expr-custom-model-data";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let source_multiplicity = payload
            .children
            .first()
            .and_then(|child| child.multiplicity);
        // Extra component patterns only exist when Bukkit exposes
        // CustomModelDataComponent, so the SSG registration already encodes that capability.
        // Skript 2.12 also changed the shared integer branch from Long to Integer even when
        // running on an older Minecraft release where only that branch is registered.
        let modern = crate::runtime::skript_at_least(2, 12).unwrap_or(payload.mark > 0);
        let Some((return_type, possible_return_types, multiplicity, mode, accepted)) =
            custom_model_data_semantics(modern, payload.mark, source_multiplicity)
        else {
            return SemanticResolution::Reject(
                "custom model data Expression has an unknown parse mark or source multiplicity"
                    .to_owned(),
            );
        };

        let contract = change_contract(&accepted, modern);
        let mut output_metadata = vec![
            metadata("semantic-mode", "custom-model-data"),
            metadata("custom-model-data-kind", mode),
        ];
        output_metadata.push(
            catalog::change_contract_metadata(&payload.registration_id, &contract)
                .expect("an in-memory custom model data contract must serialize"),
        );
        resolved_with_possible_types(
            return_type.to_owned(),
            possible_return_types
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ExpressionPossibleReturnTypesState::Complete,
            multiplicity,
            output_metadata,
        )
    })
}

type CustomModelDataSemantics = (
    &'static str,
    Vec<&'static str>,
    DynamicMultiplicity,
    &'static str,
    Vec<AcceptedChangeType>,
);

fn custom_model_data_semantics(
    modern: bool,
    mark: i32,
    source_multiplicity: Option<DynamicMultiplicity>,
) -> Option<CustomModelDataSemantics> {
    let source_multiplicity = source_multiplicity?;
    if !modern {
        return (mark == 0).then(|| {
            (
                "java.lang.Long",
                vec!["java.lang.Long"],
                source_multiplicity,
                "legacy-long",
                vec![accepted("java.lang.Number", false)],
            )
        });
    }
    let (return_type, possible, multiplicity, mode, accepted_types) = match mark {
        0 => (
            "java.lang.Integer",
            vec!["java.lang.Integer"],
            source_multiplicity,
            "integer",
            vec![accepted("java.lang.Integer", true)],
        ),
        1 => component("java.lang.Float", "floats"),
        2 => component("java.lang.Boolean", "flags"),
        3 => component("java.lang.String", "strings"),
        4 => component("ch.njol.skript.util.Color", "colors"),
        5 => {
            let types = vec![
                "java.lang.Float",
                "java.lang.Boolean",
                "java.lang.String",
                "ch.njol.skript.util.Color",
            ];
            (
                "java.lang.Object",
                types.clone(),
                DynamicMultiplicity::Multiple,
                "complete",
                types
                    .into_iter()
                    .map(|value| accepted(value, true))
                    .collect(),
            )
        }
        _ => return None,
    };
    Some((return_type, possible, multiplicity, mode, accepted_types))
}

fn component(class_name: &'static str, mode: &'static str) -> CustomModelDataSemantics {
    (
        class_name,
        vec![class_name],
        DynamicMultiplicity::Multiple,
        mode,
        vec![accepted(class_name, true)],
    )
}

fn accepted(class_name: &str, multiple: bool) -> AcceptedChangeType {
    AcceptedChangeType {
        class_name: class_name.to_owned(),
        multiple,
    }
}

fn change_contract(accepted_types: &[AcceptedChangeType], modern: bool) -> ChangeContract {
    // Skript <=2.11 returned Number.class for every ChangeMode. The component-based 2.12+
    // implementation explicitly permits ADD, REMOVE, SET, DELETE and RESET, and uses array
    // classes so plural component updates are legal.
    let modes = if modern {
        ["ADD", "REMOVE", "SET", "DELETE", "RESET"].as_slice()
    } else {
        ["ADD", "SET", "REMOVE_ALL", "REMOVE", "DELETE", "RESET"].as_slice()
    };
    ChangeContract::Resolved {
        modes: modes
            .iter()
            .map(|mode| ((*mode).to_owned(), accepted_types.to_vec()))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_212_uses_long_and_accepts_every_change_mode() {
        let (_, possible, _, mode, accepted) =
            custom_model_data_semantics(false, 0, Some(DynamicMultiplicity::Single))
                .expect("legacy custom model data must resolve");
        assert_eq!(possible, ["java.lang.Long"]);
        assert_eq!(mode, "legacy-long");
        let ChangeContract::Resolved { modes } = change_contract(&accepted, false) else {
            panic!("legacy contract must resolve");
        };
        assert_eq!(modes.len(), 6);
        assert!(!accepted[0].multiple);
    }

    #[test]
    fn post_212_integer_branch_uses_integer_array_changers() {
        let (return_type, possible, multiplicity, mode, accepted) =
            custom_model_data_semantics(true, 0, Some(DynamicMultiplicity::Single))
                .expect("integer custom model data must resolve");
        assert_eq!(return_type, "java.lang.Integer");
        assert_eq!(possible, ["java.lang.Integer"]);
        assert_eq!(multiplicity, DynamicMultiplicity::Single);
        assert_eq!(mode, "integer");
        assert_eq!(accepted[0].class_name, "java.lang.Integer");
        assert!(accepted[0].multiple);
        let ChangeContract::Resolved { modes } = change_contract(&accepted, true) else {
            panic!("modern contract must resolve");
        };
        assert_eq!(modes.len(), 5);
        assert!(!modes.contains_key("REMOVE_ALL"));
    }

    #[test]
    fn component_marks_publish_complete_possible_types_and_changers() {
        let (return_type, possible, multiplicity, mode, accepted) =
            custom_model_data_semantics(true, 5, Some(DynamicMultiplicity::Single))
                .expect("complete custom model data must resolve");
        assert_eq!(return_type, "java.lang.Object");
        assert_eq!(possible.len(), 4);
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
        assert_eq!(mode, "complete");
        assert!(accepted.iter().all(|value| value.multiple));
        let ChangeContract::Resolved { modes } = change_contract(&accepted, true) else {
            panic!("modern contract must resolve");
        };
        assert!(!modes.contains_key("REMOVE_ALL"));
    }
}
