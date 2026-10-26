use super::{SemanticResolution, matches, metadata, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};

pub(super) const HANDLER_ID: &str = "core.expression.simple-literal";
const SUPER_CLASS: &str = "ch.njol.skript.lang.util.SimpleLiteral";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Expression,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
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

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if !matches(payload, HANDLER_ID) {
        return None;
    }
    let Some(return_type) = guaranteed_class(payload.declared_return_type.as_deref()) else {
        return Some(SemanticResolution::Unresolved {
            reason: "SimpleLiteral return type is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "simple-literal")],
        });
    };
    Some(resolved(
        return_type,
        simple_literal_multiplicity(payload.declared_multiplicity),
        "simple-literal",
    ))
}

// The resolver and its editing bound share one authoritative return-class rule.
fn guaranteed_class(declared: Option<&str>) -> Option<&str> {
    declared.filter(|class| !class.trim().is_empty())
}

pub(crate) fn completion_annotations()
-> Vec<crate::nlaocs::skript_parser_addon::types::CatalogAnnotation> {
    crate::catalog::expression_bound_sources(has_simple_literal_hint)
        .as_deref()
        .map(annotations_from_source)
        .unwrap_or_default()
}

// A cheap prefilter only: even a matching description is harmless because the
// selected full record below must prove the actual superclass and declared type.
fn has_simple_literal_hint(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.contains(SUPER_CLASS))
}

fn annotations_from_source(
    bytes: &[u8],
) -> Vec<crate::nlaocs::skript_parser_addon::types::CatalogAnnotation> {
    use crate::nlaocs::skript_parser_addon::types::{CatalogAnnotation, CatalogAnnotationTarget};
    // Project only the semantic inputs. Expanding every description, tag and
    // runtime field into a Value tree exceeds the ordinary manifest fuel budget.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Source {
        super_class: Option<String>,
        return_type: Option<String>,
        registration_id: Option<String>,
    }
    let Ok(records) = serde_json::from_slice::<Vec<Source>>(bytes) else {
        return Vec::new();
    };
    if records.len() > 16_384 {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut annotations = Vec::new();
    for record in records {
        // Same direct-superclass selector as the authoritative handler. The
        // host independently binds this claim to the actual resolved handler.
        if record.super_class.as_deref() != Some(SUPER_CLASS) {
            continue;
        }
        let Some(class) = guaranteed_class(record.return_type.as_deref()) else {
            continue;
        };
        let Some(id) = record
            .registration_id
            .as_deref()
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            return Vec::new();
        }
        annotations.push(CatalogAnnotation {
            target: CatalogAnnotationTarget::Registration(id.to_owned()),
            metadata: vec![metadata(
                "parser.completion.output-bound",
                &serde_json::json!({
                    "version": 1, "handlerId": HANDLER_ID, "classes": [class],
                })
                .to_string(),
            )],
        });
    }
    annotations
}

fn simple_literal_multiplicity(declared: Option<DynamicMultiplicity>) -> DynamicMultiplicity {
    // SimpleLiteral computes isSingle() from its data length and `and` flag.
    // Without constructor metadata, the official implementation can therefore
    // produce either one value or multiple values; Both is intentional here,
    // rather than a substitute for missing type information.
    declared.unwrap_or(DynamicMultiplicity::Both)
}

#[cfg(test)]
mod tests {
    use super::{HANDLER_ID, SUPER_CLASS, register, simple_literal_multiplicity};
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, RegisteredSyntaxHandlerTarget,
    };

    #[test]
    fn bounded_prefilter_does_not_itself_grant_a_type_bound() {
        assert!(super::has_simple_literal_hint(
            br#"{"superClass":"ch.njol.skript.lang.util.SimpleLiteral","description": ["truncated"#
        ));
        let nested =
            br#"[{"description":{"superClass":"ch.njol.skript.lang.util.SimpleLiteral"}}]"#;
        assert!(super::has_simple_literal_hint(nested));
        assert!(super::annotations_from_source(nested).is_empty());
        assert!(!super::has_simple_literal_hint(
            br#"{"superClass":"ch.njol.skript.lang.util.SimpleLit"#
        ));
        assert!(!super::has_simple_literal_hint(
            br#"{"superClass":"other.Class"}"#
        ));
    }

    #[test]
    fn output_bounds_share_resolver_class_rule_and_do_not_claim_cardinality() {
        let bytes = br#"[{"registrationId":"arbitrary:constant","superClass":"ch.njol.skript.lang.util.SimpleLiteral","returnType":"example.Value","returnTypeState":"dynamic"},{"registrationId":"arbitrary:other","superClass":"example.Other","returnType":"example.Value"}]"#;
        let annotations = super::annotations_from_source(bytes);
        assert_eq!(annotations.len(), 1);
        let bound: serde_json::Value =
            serde_json::from_str(&annotations[0].metadata[0].value).unwrap();
        assert_eq!(bound["classes"], serde_json::json!(["example.Value"]));
        assert!(bound.get("multiplicity").is_none());
        assert_eq!(super::guaranteed_class(None), None);
        assert_eq!(super::guaranteed_class(Some(" ")), None);
        assert!(super::annotations_from_source(br#"[{"registrationId":"unknown","superClass":"ch.njol.skript.lang.util.SimpleLiteral"}]"#).is_empty());
        assert!(super::annotations_from_source(b"truncated").is_empty());
    }

    #[test]
    fn handler_targets_every_direct_simple_literal_subclass() {
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert_eq!(handlers[0].handler_id, HANDLER_ID);
        assert!(matches!(
            &handlers[0].targets[0],
            RegisteredSyntaxHandlerTarget::SuperClass(value) if value == SUPER_CLASS
        ));
    }

    #[test]
    fn missing_constructor_metadata_keeps_runtime_dependent_multiplicity() {
        assert_eq!(simple_literal_multiplicity(None), DynamicMultiplicity::Both);
        assert_eq!(
            simple_literal_multiplicity(Some(DynamicMultiplicity::Single)),
            DynamicMultiplicity::Single
        );
    }
}
