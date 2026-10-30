mod block_data;
mod boolean;
mod class_info;
mod color;
mod enchantment_type;
mod entity_data;
mod entity_type;
mod experience;
mod item_stack;
mod item_type;
mod number;
mod particle;
mod registered_literal;
mod storage;
mod string_literal;
mod time;
mod time_period;
mod timespan;

use crate::nlaocs::skript_parser_addon::types::{
    ExpressionLeafCandidate, ExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind, TypeParserUnresolved,
};

pub(crate) fn storage_preserves_bounds(classes: &[String]) -> bool {
    storage::preserves_bounds(classes)
}

pub(super) struct TypeParser {
    id: &'static str,
    classes: &'static [&'static str],
    parse: fn(&ExpressionPayload, &str, u64) -> Option<ExpressionLeafCandidate>,
    unresolved: Option<fn(&ExpressionPayload, &str) -> Option<TypeParserUnresolved>>,
    all_type_options: bool,
}

const PARSERS: &[TypeParser] = &[
    string_literal::PARSER,
    number::PARSER,
    boolean::PARSER,
    block_data::PARSER,
    time::PARSER,
    time_period::PARSER,
    color::PARSER,
    experience::PARSER,
    item_type::PARSER,
    item_stack::PARSER,
    entity_data::PARSER,
    entity_type::PARSER,
    enchantment_type::PARSER,
    particle::PARSER,
    timespan::PARSER,
    class_info::PARSER,
];

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    PARSERS
        .iter()
        .map(|parser| RegisteredSyntaxHandler {
            handler_id: parser.id.to_owned(),
            kind: SyntaxKind::Type,
            phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
            targets: parser
                .classes
                .iter()
                .map(|class| RegisteredSyntaxHandlerTarget::ClassSuffix((*class).to_owned()))
                .collect(),
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: if parser.all_type_options {
                vec![parser_wasm::REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()]
            } else {
                Vec::new()
            },
        })
        .collect()
}

pub(crate) fn match_type_option<'a>(
    name: &str,
    options: &'a [crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> Option<(
    &'a crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption,
    bool,
)> {
    class_info::type_option(name, options)
}

pub(crate) fn match_user_type_option(
    name: &str,
    _options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
) -> Option<(
    crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption,
    bool,
)> {
    // ClassInfo's runtime parser strips the article before matching user patterns.
    let name = crate::language::strip_indefinite_article(name.trim());
    #[cfg(target_arch = "wasm32")]
    {
        return crate::nlaocs::skript_parser_addon::catalog_data::type_for_user_input(name)
            .ok()
            .flatten()
            .map(|option| {
                let plural = class_info::is_plural_user_input(name, &option);
                (option, plural)
            });
    }
    #[cfg(not(target_arch = "wasm32"))]
    class_info::user_type_option(name, _options).map(|(option, plural)| (option.clone(), plural))
}

pub(crate) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    payload.active_type.as_ref()?;
    let constructor = standard_parser(payload);
    let mut candidate = if let Some(parser) = constructor {
        (parser.parse)(payload, text, end)
    } else {
        registered_literal::parse(payload, end)
    }?;
    annotate(payload, &mut candidate);
    // Core's fixed Literal constructors read constant data at runtime. This
    // common boundary excludes registered fallback parsers and nested evaluation;
    // it does not certify parse/init context, clone, conversion or serialization.
    if payload.active_type.as_ref().is_some_and(|active|
        active.addon_name == "Skript" && active.addon_version == "2.15.4")
        && crate::runtime::skript_version_is("2.15.4")
        && fixed_literal_evaluation(&candidate, constructor.is_some())
    {
        candidate.metadata.push(crate::public_data::preserve_local_effects());
    }
    Some(candidate)
}

pub(crate) fn is_applicable(payload: &ExpressionPayload) -> bool {
    payload.allow_literals
        && (standard_parser(payload).is_some() || registered_literal::applicable(payload))
}

pub(crate) fn unresolved(payload: &ExpressionPayload, text: &str) -> Option<TypeParserUnresolved> {
    if let Some(parser) = standard_parser(payload) {
        return (parser.unresolved?)(payload, text);
    }
    let active = payload.active_type.as_ref()?;
    if active.addon_name.eq_ignore_ascii_case("Skript")
        || !PARSERS
            .iter()
            .any(|parser| parser.classes.contains(&active.class_name.as_str()))
    {
        return None;
    }
    Some(TypeParserUnresolved {
        reason: "the Type shares a standard Java class but belongs to another addon".to_owned(),
        required_provider: Some(format!("type-parser/{}", active.registration_id)),
    })
}

fn standard_parser(payload: &ExpressionPayload) -> Option<&'static TypeParser> {
    let active = payload.active_type.as_ref()?;
    PARSERS.iter().find(|parser| {
        // The host already routes each Type registration through the declared handler targets.
        // Keep the guest boundary ownership-based so third-party Types sharing a Java class
        // remain available to their own providers without a second registration-ID gate.
        active.addon_name.eq_ignore_ascii_case("Skript")
            && parser.classes.contains(&active.class_name.as_str())
    })
}

/// Skript recognizes quoted VariableString before requested-type conversion.
/// Recover that evidence only when the ordinary typed route cannot accept it.
/// Unknown conversion information, restricted parse modes and compatible unions
/// stay on their existing route; this never supplies a new accepted expression.
pub(crate) fn diagnostic_string_payload(payload: &ExpressionPayload) -> Option<ExpressionPayload> {
    if payload.active_type.is_some()
        || !payload.allow_literals
        || !payload.allow_expressions
        || payload.expected_types.is_empty()
    {
        return None;
    }
    if !payload.candidate_ends.iter().any(|end| {
        crate::expression_candidates::expression_slice(payload, *end)
            .and_then(|text| string_literal::parse(payload, text, *end))
            .is_some()
    }) {
        return None;
    }
    if !payload.expected_types.iter().all(|expected| {
        matches!(
            crate::catalog::can_convert("java.lang.String", &expected.class_name),
            Ok(crate::catalog::TypeRelation::Incompatible)
        )
    }) {
        return None;
    }
    let active = crate::catalog::type_for_class("java.lang.String").ok()??;
    let mut typed = payload.clone();
    typed.active_type = Some(active);
    parses_string(&typed).then_some(typed)
}

pub(crate) fn parses_string(payload: &ExpressionPayload) -> bool {
    standard_parser(payload).is_some_and(|parser| parser.id == string_literal::PARSER.id)
}

pub(crate) fn annotate(payload: &ExpressionPayload, candidate: &mut ExpressionLeafCandidate) {
    let Some(active) = &payload.active_type else {
        return;
    };
    // These stronger witnesses still apply only to the audited primitive storage
    // constructors, not to every side-effect-free literal (for example Timespan).
    if active.addon_name == "Skript"
        && active.addon_version == "2.15.4"
        && crate::runtime::skript_version_is("2.15.4")
        && fixed_literal_evaluation(candidate, standard_parser(payload).is_some())
        && matches!(candidate.parser_id.as_str(),
            "core.literal.number" | "core.literal.boolean" | "core.literal.string")
    {
        if crate::catalog::standard_primitive_parser(active)
            && candidate.return_type.as_ref().is_some_and(|class| {
                crate::catalog::stringification_preserving_classes().contains(class)
                    && crate::catalog::type_init_has_no_changer(class)
            })
        {
            candidate
                .metadata
                .push(crate::public_data::preserve_evaluation_context());
        }
        candidate
            .metadata
            .push(crate::public_data::non_empty_value_result());
        candidate
            .metadata
            .push(crate::public_data::default_clone_change_input());
    }
    candidate.metadata.extend([
        crate::expression_candidates::metadata("type-parser-definition-id", &active.definition_id),
        crate::expression_candidates::metadata(
            "type-parser-registration-id",
            &active.registration_id,
        ),
        crate::expression_candidates::metadata("type-parser-code-name", &active.code_name),
    ]);
}

// The construction route, not the public parser ID or Literal tag alone,
// identifies our fixed constructors. Registered fallback stays unproven.
fn fixed_literal_evaluation(candidate: &ExpressionLeafCandidate, core_constructor: bool) -> bool {
    core_constructor
        && candidate.kind == crate::nlaocs::skript_parser_addon::types::ExpressionLeafKind::Literal
        && candidate.children.is_empty()
        && !candidate.public_data.iter().any(|data|
            data.schema_id == crate::public_data::INTERPOLATION_SCHEMA_ID)
}

#[cfg(test)]
mod literal_binding_evaluation_tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity, ExpressionLeafKind};

    #[test]
    fn fixed_duration_evaluation_is_not_arbitrary_parser_or_storage_proof() {
        let mut value=crate::expression_candidates::candidate("core.literal.timespan",
            ExpressionLeafKind::Literal,0,6,"ch.njol.skript.util.Timespan",DynamicMultiplicity::Single);
        assert!(fixed_literal_evaluation(&value,true));
        assert!(value.metadata.is_empty(),"evaluation eligibility alone publishes no storage/context witness");
        assert!(!fixed_literal_evaluation(&value,false),"a registered parser is not a fixed constructor");
        value.parser_id="core.literal.time".into();
        assert!(fixed_literal_evaluation(&value,true),"the contract is not a duration special case");
        value.children.push(crate::nlaocs::skript_parser_addon::types::ParseResultReference {host_token:1,root_id:0});
        assert!(!fixed_literal_evaluation(&value,true),"child evaluation requires its own contract");
        value.children.clear();
        value.public_data.push(crate::nlaocs::skript_parser_addon::types::ExpressionPublicData {
            schema_id:crate::public_data::INTERPOLATION_SCHEMA_ID.into(),schema_version:1,json:"{}".into()});
        assert!(!fixed_literal_evaluation(&value,true),"interpolation is evaluated separately");
        value.public_data.clear();value.kind=ExpressionLeafKind::Variable;
        assert!(!fixed_literal_evaluation(&value,true));
    }
}
