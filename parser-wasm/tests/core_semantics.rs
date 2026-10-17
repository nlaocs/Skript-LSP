//! Branch checks use the shipped component, including registration bindings and
//! catalog imports. Source parsing and REPL tests exercise the separate callers.

use parser_wasm::bindings::nlaocs::skript_parser_addon::types::*;
use parser_wasm::host::{
    DispatchRequest, DispatchResult, DispatchTarget, HostConfig, HostError, ParserHost,
    RuntimeProfile,
};
use std::path::Path;
use std::sync::Arc;
use syntaxes::Catalog;

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const DELAY: &str = "parser.delay-state";
const DEATH: &str = "org.bukkit.event.entity.EntityDeathEvent";
const JOIN: &str = "org.bukkit.event.player.PlayerJoinEvent";
const RESPAWN: &str = "org.bukkit.event.player.PlayerRespawnEvent";
const SWIM: &str = "org.bukkit.event.entity.EntityToggleSwimEvent";
const SYNTAX_CONTEXT: u64 = 73;

struct Fixture {
    host: ParserHost,
    catalog: Arc<Catalog>,
}

impl Fixture {
    fn new(version: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/type-parser-versions/skript-2.15.4");
        let catalog = Arc::new(ssg::load(path).unwrap().catalog().clone());
        let host = ParserHost::new(
            CORE_LIBRARY,
            HostConfig {
                syntax_catalog: Some(catalog.clone()),
                runtime_profile: RuntimeProfile {
                    skript_version: Some(version.to_owned()),
                    ..RuntimeProfile::default()
                },
                ..HostConfig::default()
            },
        )
        .expect("real CoreLibrary must initialize");
        Self { host, catalog }
    }

    fn effect(&self, suffix: &str, pattern: u64, mark: i32, value: Option<&str>) -> EffectPayload {
        let common = &self
            .catalog
            .effects()
            .find(|effect| effect.common.element_class.as_str().ends_with(suffix))
            .expect("fixture Effect")
            .common;
        EffectPayload {
            input: "fixture semantic input".to_owned(),
            context: parse_context(),
            raw_node_id: 7,
            span: span(),
            timing: EffectTiming::After,
            candidate: Some(EffectCandidate {
                raw_node_id: 7,
                definition_id: common.definition_id.as_str().to_owned(),
                registration_id: common.registration_id.as_str().to_owned(),
                element_class: Some(common.element_class.as_str().to_owned()),
                priority: 0,
                registration_order: common.registration_order as u64,
                pattern_index: pattern,
                pattern: common
                    .patterns
                    .get(pattern as usize)
                    .unwrap_or(&common.patterns[0])
                    .source
                    .clone(),
                span: span(),
                captures: value
                    .into_iter()
                    .map(|value| {
                        EffectCapture::Regex(EffectRegexCapture {
                            pattern_span: TextRange { start: 0, end: 1 },
                            value: value.to_owned(),
                            span: span(),
                        })
                    })
                    .collect(),
                tags: Vec::new(),
                mark,
                marks: Vec::new(),
                handler: None,
                metadata: Vec::new(),
                parsed_captures: Vec::new(),
            }),
            alternatives: Vec::new(),
            failure: None,
            near_match: None,
        }
    }

    fn filter(&self) -> SectionPayload {
        let common = &self
            .catalog
            .sections()
            .find(|section| {
                section
                    .common
                    .element_class
                    .as_str()
                    .ends_with(".SecFilter")
            })
            .expect("fixture SecFilter")
            .common;
        SectionPayload {
            input: "filter {_list::*} to match".to_owned(),
            context: parse_context(),
            raw_node_id: 7,
            span: span(),
            timing: SectionTiming::EnterChildren,
            preceding_siblings: Vec::new(),
            next_sibling: None,
            raw_children: vec![raw_child(SectionRawNodeKind::Simple)],
            candidate: SectionCandidate {
                raw_node_id: 7,
                definition_id: common.definition_id.as_str().to_owned(),
                registration_id: common.registration_id.as_str().to_owned(),
                element_class: Some(common.element_class.as_str().to_owned()),
                pattern_index: 0,
                span: span(),
                loop_section: false,
                effect_section: false,
                section_expression: false,
                regex_captures: Vec::new(),
                tags: Vec::new(),
                mark: 0,
                marks: Vec::new(),
                parsed_captures: vec![ParsedCapture {
                    capture_index: 0,
                    parser_id: "host.expression".to_owned(),
                    status: ParseResultStatus::Success,
                    text: "{_list::*}".to_owned(),
                    span: span(),
                    expected_types: Vec::new(),
                    summary: Some(ParseSummary {
                        kind: "variable".to_owned(),
                        definition_id: None,
                        registration_id: None,
                        element_class: None,
                        pattern_index: None,
                        return_type: Some("java.lang.Object".to_owned()),
                        possible_return_types: vec![
                            "java.lang.Long".to_owned(),
                            "java.lang.String".to_owned(),
                        ],
                        possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                        multiplicity: Some(DynamicMultiplicity::Multiple),
                        default_expression: None,
                        public_data: Vec::new(),
                        metadata: Vec::new(),
                    }),
                    attachments: Vec::new(),
                    diagnostics: Vec::new(),
                }],
                body_mode: SectionBodyMode::Trigger,
                metadata: Vec::new(),
            },
        }
    }

    fn run(&mut self, payload: HookPayload) -> DispatchResult {
        let (phase, kind, subscription) = match &payload {
            HookPayload::Effect(_) => (
                HookPhase::Effect,
                SyntaxKind::Effect,
                "core.effect-semantics",
            ),
            HookPayload::Section(_) => (
                HookPhase::Section,
                SyntaxKind::Section,
                "core.section-semantics",
            ),
            _ => panic!("Effect or Section required"),
        };
        let result = self
            .host
            .dispatch(
                "file:///workspace",
                DispatchRequest {
                    context: InvocationContext {
                        invocation_id: 1,
                        subscription_id: String::new(),
                        document_id: "file:///core-semantics.sk".to_owned(),
                        document_revision: 1,
                        expansion: None,
                        syntax_context: SYNTAX_CONTEXT,
                    },
                    target: DispatchTarget::SyntaxKind(kind),
                    phase,
                    payload,
                },
            )
            .expect("component dispatch must succeed");
        assert!(result.fault.is_none(), "{result:#?}");
        assert!(result.failures.is_empty(), "{result:#?}");
        assert_eq!(result.calls.len(), 1, "{result:#?}");
        assert_eq!(result.calls[0].component_id, "nlaocs.core-library");
        assert_eq!(result.calls[0].subscription_id, subscription);
        assert!(result.effects.parse_requests.is_empty());
        result
    }
}

fn span() -> MappedSpan {
    MappedSpan {
        virtual_range: TextRange { start: 7, end: 15 },
        origins: vec![SourceOrigin {
            original_range: TextRange { start: 27, end: 35 },
            kind: OriginKind::Exact,
            expansion: None,
        }],
    }
}

fn assert_span(actual: &MappedSpan) {
    let expected = span();
    assert_eq!(actual.virtual_range.start, expected.virtual_range.start);
    assert_eq!(actual.virtual_range.end, expected.virtual_range.end);
    assert_eq!(actual.origins.len(), expected.origins.len());
    for (actual, expected) in actual.origins.iter().zip(expected.origins) {
        assert_eq!(actual.original_range.start, expected.original_range.start);
        assert_eq!(actual.original_range.end, expected.original_range.end);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.expansion, expected.expansion);
    }
}

fn parse_context() -> ParseContext {
    ParseContext {
        syntax_context: SYNTAX_CONTEXT,
        event_classes: Vec::new(),
        section_stack: Vec::new(),
        values: Vec::new(),
    }
}

fn frame(class: &str, loop_section: bool, metadata: &[(&str, &str)]) -> SectionScopeFrame {
    SectionScopeFrame {
        scope_id: 1,
        parent_scope_id: None,
        raw_node_id: 1,
        kind: SectionScopeKind::Section,
        definition_id: "section:fixture".to_owned(),
        registration_id: "section:fixture:0".to_owned(),
        element_class: Some(class.to_owned()),
        pattern_index: 0,
        pattern: String::new(),
        source: String::new(),
        addon_name: None,
        addon_version: None,
        owner_component_id: None,
        loop_section,
        effect_section: false,
        section_expression: false,
        captures: Vec::new(),
        metadata: metadata
            .iter()
            .map(|(key, value)| MetadataEntry {
                key: (*key).to_owned(),
                value: (*value).to_owned(),
                owner_component_id: None,
            })
            .collect(),
    }
}

fn mixed_stack() -> Vec<SectionScopeFrame> {
    vec![
        frame("fixture.Loop", true, &[]),
        frame("ch.njol.skript.sections.SecConditional", false, &[]),
        frame("fixture.While", false, &[("while-loop-control", "true")]),
        frame(
            "fixture.Conditional",
            false,
            &[("semantic-mode", "conditional")],
        ),
        frame("ch.njol.skript.sections.SecConditional", false, &[]),
        frame("fixture.Other", false, &[]),
    ]
}

fn event_context(payload: &mut EffectPayload, events: &[&str], delayed: Option<&str>) {
    payload.context.event_classes = events.iter().map(|value| (*value).to_owned()).collect();
    payload.context.values = delayed
        .into_iter()
        .map(|value| ParseContextValue {
            key: DELAY.to_owned(),
            value: value.to_owned(),
        })
        .collect();
}

fn metadata<'a>(entries: &'a [MetadataEntry], key: &str) -> Option<&'a str> {
    let values = entries
        .iter()
        .filter(|entry| {
            entry.key == key && entry.owner_component_id.as_deref() == Some("nlaocs.core-library")
        })
        .collect::<Vec<_>>();
    assert!(
        values.len() <= 1,
        "duplicate owned metadata {key}: {entries:#?}"
    );
    values.first().map(|entry| entry.value.as_str())
}

fn effect_metadata<'a>(result: &'a DispatchResult, key: &str) -> Option<&'a str> {
    let HookPayload::Effect(payload) = &result.payload else {
        panic!("Effect payload");
    };
    metadata(&payload.candidate.as_ref().unwrap().metadata, key)
}

fn accepted(result: &DispatchResult) {
    assert!(
        matches!(result.decision, HookDecision::ContinueProcessing),
        "{result:#?}"
    );
}

fn rejected(result: &DispatchResult, reason: &str, code: Option<&str>) {
    let HookDecision::Reject(rejection) = &result.decision else {
        panic!("expected Reject: {result:#?}");
    };
    assert!(rejection.reason.contains(reason), "{rejection:#?}");
    assert!(result.effects.context_updates.is_empty());
    assert!(result.effects.diagnostics.is_empty());
    if let Some(code) = code {
        assert_eq!(rejection.diagnostics.len(), 1);
        let diagnostic = &rejection.diagnostics[0];
        assert_eq!(diagnostic.code, code);
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_span(&diagnostic.span);
    }
}

fn warnings(result: &DispatchResult, codes: &[&str]) {
    assert_eq!(
        result
            .effects
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        codes,
        "{result:#?}"
    );
    for diagnostic in &result.effects.diagnostics {
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
        assert_span(&diagnostic.span);
    }
    assert_eq!(
        effect_metadata(result, "semantic-state"),
        (!codes.is_empty()).then_some("unresolved")
    );
    assert_eq!(
        effect_metadata(result, "semantic-unresolved"),
        codes.last().copied()
    );
}

#[test]
fn continue_uses_real_loop_depth_and_rejects_invalid_ordinals() {
    let mut fixture = Fixture::new("2.15.4");
    for (pattern, value, ordinal) in [(0, None, "2"), (1, Some("1"), "1"), (1, Some("2"), "2")] {
        let mut payload = fixture.effect(".EffContinue", pattern, 0, value);
        payload.context.section_stack = mixed_stack();
        let result = fixture.run(HookPayload::Effect(payload));
        accepted(&result);
        assert_eq!(
            effect_metadata(&result, "semantic-mode"),
            Some("continue-loop")
        );
        assert_eq!(effect_metadata(&result, "loop-ordinal"), Some(ordinal));
        assert_eq!(effect_metadata(&result, "available-loop-depth"), Some("2"));
        warnings(&result, &[]);
    }
    for (pattern, value, reason) in [
        (1, Some("0"), "positive loop ordinal"),
        (1, Some("-1"), "positive loop ordinal"),
        (1, Some("not a number"), "positive loop ordinal"),
        (1, Some("18446744073709551616"), "positive loop ordinal"),
        (1, None, "positive loop ordinal"),
        (1, Some("3"), "only 2 loop(s)"),
        (2, None, "unknown pattern index"),
    ] {
        let mut payload = fixture.effect(".EffContinue", pattern, 0, value);
        payload.context.section_stack = mixed_stack();
        rejected(&fixture.run(HookPayload::Effect(payload)), reason, None);
    }
    let mut payload = fixture.effect(".EffContinue", 0, 0, None);
    payload.context.section_stack = vec![frame("fixture.Other", false, &[])];
    rejected(
        &fixture.run(HookPayload::Effect(payload)),
        "only be used in loops",
        None,
    );
}

#[test]
fn exit_uses_target_specific_depth_and_checks_all_count_patterns() {
    let mut fixture = Fixture::new("2.15.4");
    let result = fixture.run(HookPayload::Effect(fixture.effect(".EffExit", 0, 0, None)));
    accepted(&result);
    assert_eq!(effect_metadata(&result, "exit-target"), Some("trigger"));
    assert_eq!(effect_metadata(&result, "exit-count"), Some("1"));
    assert_eq!(
        effect_metadata(&result, "available-target-depth"),
        Some("1")
    );
    for (mark, target, depth) in [
        (0, "sections", 6_u64),
        (1, "loops", 2),
        (2, "conditionals", 3),
    ] {
        for (pattern, value, count) in [
            (1, None, 1),
            (2, Some(depth.to_string()), depth),
            (3, None, depth),
        ] {
            let mut payload = fixture.effect(".EffExit", pattern, mark, value.as_deref());
            payload.context.section_stack = mixed_stack();
            let result = fixture.run(HookPayload::Effect(payload));
            accepted(&result);
            assert_eq!(effect_metadata(&result, "semantic-mode"), Some("exit"));
            assert_eq!(effect_metadata(&result, "exit-target"), Some(target));
            assert_eq!(
                effect_metadata(&result, "exit-count"),
                Some(count.to_string().as_str())
            );
            assert_eq!(
                effect_metadata(&result, "available-target-depth"),
                Some(depth.to_string().as_str())
            );
            warnings(&result, &[]);
        }
        for value in ["0".to_owned(), (depth + 1).to_string()] {
            let mut payload = fixture.effect(".EffExit", 2, mark, Some(&value));
            payload.context.section_stack = mixed_stack();
            rejected(
                &fixture.run(HookPayload::Effect(payload)),
                if value == "0" {
                    "positive Section count"
                } else {
                    "only"
                },
                None,
            );
        }
        for pattern in [1, 3] {
            let payload = fixture.effect(".EffExit", pattern, mark, None);
            rejected(
                &fixture.run(HookPayload::Effect(payload)),
                "only 0 are present",
                None,
            );
        }
    }
    for (pattern, mark, value, reason) in [
        (1, 3, None, "invalid target mark"),
        (4, 0, None, "unknown pattern index"),
        (2, 0, None, "positive Section count"),
        (2, 0, Some("no"), "positive Section count"),
        (2, 0, Some("18446744073709551616"), "positive Section count"),
    ] {
        let mut payload = fixture.effect(".EffExit", pattern, mark, value);
        payload.context.section_stack = mixed_stack();
        rejected(&fixture.run(HookPayload::Effect(payload)), reason, None);
    }
}

#[test]
fn cancel_modes_validate_event_hierarchy_delay_and_diagnostics() {
    let mut fixture = Fixture::new("2.15.4");
    for (pattern, mode) in [(0, "cancel-event"), (1, "uncancel-event")] {
        for events in [
            vec![DEATH],
            vec!["org.bukkit.event.block.BlockCanBuildEvent"],
            vec!["fixture.Unknown", DEATH],
        ] {
            let mut payload = fixture.effect(".EffCancelEvent", pattern, 0, None);
            event_context(&mut payload, &events, Some("false"));
            let result = fixture.run(HookPayload::Effect(payload));
            accepted(&result);
            assert_eq!(effect_metadata(&result, "semantic-mode"), Some(mode));
            warnings(&result, &[]);
        }
    }
    for (pattern, events, delayed, code, reason) in [
        (
            2,
            vec![DEATH],
            Some("false"),
            "unknown-pattern",
            "unknown pattern index",
        ),
        (0, vec![], Some("false"), "outside-event", "inside an Event"),
        (
            0,
            vec![DEATH],
            Some("true"),
            "after-delay",
            "already passed",
        ),
        (
            1,
            vec![DEATH],
            Some("true"),
            "after-delay",
            "already passed",
        ),
        (
            0,
            vec![JOIN],
            Some("false"),
            "unsupported-event",
            "cannot be cancelled",
        ),
        (
            0,
            vec!["org.bukkit.event.player.PlayerLoginEvent"],
            Some("false"),
            "unsupported-event",
            "kick the player instead",
        ),
    ] {
        let mut payload = fixture.effect(".EffCancelEvent", pattern, 0, None);
        event_context(&mut payload, &events, delayed);
        rejected(
            &fixture.run(HookPayload::Effect(payload)),
            reason,
            Some(&format!("core.eff-cancel-event.{code}")),
        );
    }
    for delayed in [None, Some("unknown")] {
        let mut payload = fixture.effect(".EffCancelEvent", 0, 0, None);
        event_context(&mut payload, &[DEATH], delayed);
        let result = fixture.run(HookPayload::Effect(payload));
        accepted(&result);
        warnings(&result, &["core.eff-cancel-event.unresolved-delay-state"]);
    }
    let mut payload = fixture.effect(".EffCancelEvent", 0, 0, None);
    event_context(&mut payload, &["fixture.Unknown"], Some("false"));
    let result = fixture.run(HookPayload::Effect(payload));
    accepted(&result);
    warnings(&result, &["core.eff-cancel-event.unresolved-event-type"]);
}

#[test]
fn toggle_swim_rule_uses_the_real_runtime_version_and_cancel_mode() {
    for (version, pattern, rejects) in [
        ("2.8.7", 0, false),
        ("2.9.0", 0, true),
        ("2.15.4", 1, false),
    ] {
        let mut fixture = Fixture::new(version);
        let mut payload = fixture.effect(".EffCancelEvent", pattern, 0, None);
        event_context(&mut payload, &[SWIM], Some("false"));
        let result = fixture.run(HookPayload::Effect(payload));
        if rejects {
            rejected(
                &result,
                "toggle swim",
                Some("core.eff-cancel-event.toggle-swim"),
            );
        } else {
            accepted(&result);
            assert_eq!(
                effect_metadata(&result, "semantic-mode"),
                Some(if pattern == 0 {
                    "cancel-event"
                } else {
                    "uncancel-event"
                })
            );
            warnings(&result, &[]);
        }
    }
}

#[test]
fn respawn_distinguishes_death_delay_unknown_and_forbidden_event() {
    let mut fixture = Fixture::new("2.15.4");
    for (events, delayed, force_delay, codes) in [
        (vec![DEATH], Some("false"), Some("true"), vec![]),
        (vec![DEATH], Some("true"), Some("false"), vec![]),
        (
            vec![DEATH],
            None,
            None,
            vec!["core.eff-respawn.unresolved-delay-state"],
        ),
        (vec![JOIN], Some("false"), Some("false"), vec![]),
        (vec![], None, Some("false"), vec![]),
        (
            vec!["fixture.Unknown"],
            Some("false"),
            None,
            vec![
                "core.eff-respawn.unresolved-event-type",
                "core.eff-respawn.unresolved-death-event",
            ],
        ),
    ] {
        let mut payload = fixture.effect(".EffRespawn", 0, 0, None);
        event_context(&mut payload, &events, delayed);
        let result = fixture.run(HookPayload::Effect(payload));
        accepted(&result);
        assert_eq!(
            effect_metadata(&result, "semantic-mode"),
            Some("force-respawn")
        );
        assert_eq!(effect_metadata(&result, "respawn-force-delay"), force_delay);
        warnings(&result, &codes);
    }
    let mut payload = fixture.effect(".EffRespawn", 0, 0, None);
    event_context(&mut payload, &[RESPAWN], Some("false"));
    rejected(
        &fixture.run(HookPayload::Effect(payload)),
        "respawn Event",
        Some("core.eff-respawn.respawn-event"),
    );
}

fn raw_child(kind: SectionRawNodeKind) -> SectionRawNode {
    SectionRawNode {
        raw_node_id: 8,
        kind,
        source: "fixture condition".to_owned(),
        span: span(),
    }
}

#[test]
fn filter_tags_select_metadata_body_mode_and_scoped_input_updates() {
    let mut fixture = Fixture::new("2.15.4");
    for (tag, mode) in [
        (None, "all"),
        (Some("all"), "all"),
        (Some("any"), "any"),
        (Some("AnY"), "any"),
    ] {
        let mut payload = fixture.filter();
        payload.candidate.tags = tag
            .into_iter()
            .map(|value| SyntaxTag {
                value: value.to_owned(),
                pattern_span: TextRange { start: 0, end: 1 },
                input_span: span(),
                implicit: false,
            })
            .collect();
        let result = fixture.run(HookPayload::Section(payload));
        accepted(&result);
        assert!(result.effects.diagnostics.is_empty());
        let HookPayload::Section(payload) = &result.payload else {
            panic!("Section payload")
        };
        assert_eq!(payload.candidate.body_mode, SectionBodyMode::Conditions);
        assert_eq!(
            metadata(&payload.candidate.metadata, "semantic-mode"),
            Some("filter-section")
        );
        assert_eq!(
            metadata(&payload.candidate.metadata, "filter-mode"),
            Some(mode)
        );
        assert_eq!(
            metadata(&payload.candidate.metadata, "input-source.has-indices"),
            Some("true")
        );
        let updates = result
            .effects
            .context_updates
            .iter()
            .map(|u| {
                assert_eq!(u.syntax_context, SYNTAX_CONTEXT);
                (
                    u.key.as_str(),
                    std::str::from_utf8(u.value.as_deref().expect("set context value")).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                ("core.section.class", "ch.njol.skript.sections.SecFilter"),
                ("core.section.depth", "1"),
                ("core.input-source.available", "true"),
                ("core.input-source.has-indices", "true"),
                (
                    "core.input-source.value-types",
                    "java.lang.Long;java.lang.String"
                ),
                ("core.section.filter", "true"),
                ("core.section.filter.mode", mode),
            ]
        );
    }
    let mut payload = fixture.filter();
    payload.timing = SectionTiming::ExitChildren;
    payload.raw_children.clear();
    payload.candidate.parsed_captures.clear();
    let result = fixture.run(HookPayload::Section(payload));
    accepted(&result);
    assert!(result.effects.context_updates.is_empty());
    assert!(result.effects.diagnostics.is_empty());
    let HookPayload::Section(payload) = result.payload else {
        panic!("Section payload")
    };
    assert_eq!(payload.candidate.body_mode, SectionBodyMode::Conditions);
    assert_eq!(metadata(&payload.candidate.metadata, "filter-mode"), None);
}

#[test]
fn filter_rejects_invalid_bodies_and_preserves_unresolved_source_information() {
    let mut fixture = Fixture::new("2.15.4");
    for (children, reason) in [
        (vec![], "at least one condition"),
        (
            vec![SectionRawNodeKind::Blank, SectionRawNodeKind::Comment],
            "at least one condition",
        ),
        (
            vec![SectionRawNodeKind::Section],
            "may not contain other sections",
        ),
        (
            vec![SectionRawNodeKind::Invalid],
            "only contain simple conditions",
        ),
    ] {
        let mut payload = fixture.filter();
        payload.raw_children = children.into_iter().map(raw_child).collect();
        rejected(&fixture.run(HookPayload::Section(payload)), reason, None);
    }
    for (kind, multiplicity, rejects) in [
        ("literal", Some(DynamicMultiplicity::Multiple), true),
        ("variable", Some(DynamicMultiplicity::Single), true),
        ("variable", Some(DynamicMultiplicity::Both), false),
        ("variable", None, false),
    ] {
        let mut payload = fixture.filter();
        let summary = payload.candidate.parsed_captures[0]
            .summary
            .as_mut()
            .unwrap();
        summary.kind = kind.to_owned();
        summary.multiplicity = multiplicity;
        let result = fixture.run(HookPayload::Section(payload));
        if rejects {
            rejected(&result, "only filter list variables", None);
        } else {
            accepted(&result);
            assert!(result.effects.context_updates.is_empty());
            assert_eq!(result.effects.diagnostics.len(), 1);
            assert_eq!(
                result.effects.diagnostics[0].code,
                "core.sec-filter.unresolved-multiplicity"
            );
            assert_span(&result.effects.diagnostics[0].span);
            let HookPayload::Section(payload) = result.payload else {
                panic!("Section payload")
            };
            assert_eq!(
                metadata(&payload.candidate.metadata, "semantic-state"),
                Some("unresolved")
            );
            assert_eq!(metadata(&payload.candidate.metadata, "filter-mode"), None);
        }
    }
}

#[test]
fn filter_introduction_boundary_is_checked_by_the_component() {
    for (version, supported) in [("2.9.5", false), ("2.10.0", true), ("2.16.0", true)] {
        let mut fixture = Fixture::new(version);
        let result = fixture.run(HookPayload::Section(fixture.filter()));
        if supported {
            accepted(&result);
            let HookPayload::Section(payload) = result.payload else {
                panic!("Section payload")
            };
            assert_eq!(
                metadata(&payload.candidate.metadata, "filter-mode"),
                Some("all")
            );
        } else {
            rejected(&result, "before Skript 2.10", None);
        }
    }
}

#[test]
fn missing_malformed_and_future_versions_are_rejected_before_semantic_dispatch() {
    // A Catalog would fill a missing version from its runtime profile.
    for version in [None, Some("unknown"), Some("2.17.0")] {
        let error = ParserHost::new(
            CORE_LIBRARY,
            HostConfig {
                runtime_profile: RuntimeProfile {
                    skript_version: version.map(str::to_owned),
                    ..RuntimeProfile::default()
                },
                ..HostConfig::default()
            },
        )
        .err()
        .expect("invalid runtime must not reach semantic dispatch");
        assert!(
            matches!(error, HostError::InitializationRejected { ref component_id, .. } if component_id == "nlaocs.core-library"),
            "{error:#?}"
        );
        assert!(
            error.to_string().contains(if version.is_none() {
                "requires the Skript version"
            } else if version == Some("unknown") {
                "cannot parse"
            } else {
                "supports Skript 2.6.4 through 2.16.x"
            }),
            "{error}"
        );
    }
}
