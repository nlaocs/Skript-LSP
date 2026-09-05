use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    EffectParseRequest, EffectParserConfig, ExpressionExpectedType, ExpressionListConjunction,
    ExpressionNodeKind, ExpressionParseContext, ExpressionParseRequest, ExpressionParserConfig,
    FailureTrace, MappedSource, MatchSyntaxKind, ParsedCaptureStatus, ParsedCaptureValue,
    PatternFailureReason, RawTreeOptions, TextRange, parse_raw_tree,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{
    Catalog, CatalogParts, ClassName, FunctionParameter, Multiplicity, PossibleReturnTypesState,
    RegistrationId, ReturnTypeState, Syntax, TypeLiteral,
};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const EFFECT_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/effect-addon.wasm"
));
const DYNAMIC_SYNTAX_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/dynamic-syntax-addon.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn effect_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| match syntax {
            Syntax::Type(value) => matches!(value.code_name.as_str(), "string" | "object"),
            Syntax::Effect(value) => {
                value.common.definition_id.as_str()
                    == "effect:skript:751b28432979bd1f00e370ffe6f6c3279e4936b90071eda5ed732d7cda2c0504"
                    || value.common.patterns.iter().any(|pattern| {
                        matches!(
                            pattern.source.as_str(),
                            "dummy effect registered through wrapper"
                                | "run dummy fixture effect [with %-string%]"
                        )
                    })
            }
            _ => false,
        })
        .cloned()
        .collect();
    Arc::new(
        Catalog::new(CatalogParts {
            syntaxes,
            converters: Vec::new(),
            comparators: Vec::new(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: Vec::new(),
            operations: BTreeMap::new(),
            differences: Vec::new(),
            classes: Vec::new(),
            aliases: source.aliases().clone(),
            plural_rules: source.plural_rules().clone(),
            language: source
                .language_entries()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
        })
        .with_unchecked_source(source_view),
    )
}

fn full_dynamic_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let mut syntaxes = source.syntaxes().to_vec();
    for syntax in &mut syntaxes {
        if let Syntax::Type(value) = syntax
            && value.code_name.as_str() == "entitydata"
        {
            // A freshly generated SSG snapshot obtains this finite value from ClassInfo.supplier.
            value.type_literals.push(TypeLiteral {
                text: "zombie".to_owned(),
                plural_text: Some("zombies".to_owned()),
                variable_name: Some("entitydata:zombie".to_owned()),
                debug_text: None,
                value_class: ClassName("ch.njol.skript.entity.SimpleEntityData".to_owned()),
                represented_class: Some(ClassName("org.bukkit.entity.Zombie".to_owned())),
                enum_constant: None,
            });
            value.type_literals.push(TypeLiteral {
                text: "player".to_owned(),
                plural_text: Some("players".to_owned()),
                variable_name: Some("entitydata:player".to_owned()),
                debug_text: None,
                value_class: ClassName("ch.njol.skript.entity.SimpleEntityData".to_owned()),
                represented_class: Some(ClassName("org.bukkit.entity.Player".to_owned())),
                enum_constant: None,
            });
        }
        let Syntax::Expression(expression) = syntax else {
            continue;
        };
        let element_class = expression.common.element_class.as_str();
        if element_class.ends_with(".PropExprSize") {
            expression.return_type_state = ReturnTypeState::Dynamic;
            expression.possible_return_types = vec![ClassName("java.lang.Long".to_owned())];
            expression.possible_return_types_state = PossibleReturnTypesState::Partial;
        } else if element_class.ends_with(".ExprParse") {
            // Schema 4 may retain Object as a partial possible return type even
            // when the registered WASM handler can narrow the value from its
            // ClassInfo capture. This mirrors a fresh SSG snapshot.
            expression.return_type_state = ReturnTypeState::Dynamic;
            expression.possible_return_types = vec![ClassName("java.lang.Object".to_owned())];
            expression.possible_return_types_state = PossibleReturnTypesState::Partial;
        } else if element_class.ends_with(".ExprEntities") {
            expression.return_type_state = ReturnTypeState::Dynamic;
            expression.possible_return_types.clear();
            expression.possible_return_types_state = PossibleReturnTypesState::Unresolved;
        }
    }
    Arc::new(
        Catalog::new(CatalogParts {
            syntaxes,
            converters: source.converters().to_vec(),
            comparators: source.comparators().to_vec(),
            event_values: source.event_values().to_vec(),
            properties: source.properties().to_vec(),
            operators: source.operators().to_vec(),
            operations: source.operations().clone(),
            differences: source.differences().to_vec(),
            classes: source.classes().to_vec(),
            aliases: source.aliases().clone(),
            plural_rules: source.plural_rules().clone(),
            language: source
                .language_entries()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
        })
        .with_unchecked_source(source_view),
    )
}

#[test]
fn core_library_parses_boolean_alias_and_supplied_type_literals() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 12)
        .unwrap();

    for source in [
        "send 2 if true is true",
        "send stone",
        "send zombie",
        "send all players",
    ] {
        let result = parse_effect(&mut host, &transaction, 12, source);
        assert!(
            result.matches.selected.is_some(),
            "{source:?} must parse; unknown result: {:#?}",
            result.matches.unknown
        );
    }

    let invalid_comparison = parse_effect(&mut host, &transaction, 12, "send 1 if 1 is true");
    assert!(invalid_comparison.matches.selected.is_none());
    let failure = format!("{:#?}", invalid_comparison.matches.unknown);
    assert!(
        failure.contains("cannot compare java.lang.Long with java.lang.Boolean"),
        "native Skript rejects the same incompatible comparison: {failure}"
    );
    transaction.cancel().unwrap();
}

#[test]
fn common_effects_parse_with_collection_and_nested_expression_inputs() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 14)
        .unwrap();

    for source in [
        "broadcast stone",
        "send 1 to all players",
        "teleport all players to location(1,2,3)",
        "set slot 0 of a random element out of all players to 2 stone",
    ] {
        let result = parse_effect(&mut host, &transaction, 14, source);
        assert!(
            result.matches.selected.is_some(),
            "known Skript Effect must parse: {source:?}; failure: {:#?}",
            result.matches.unknown
        );
    }

    transaction.cancel().unwrap();
}

#[test]
fn real_effect_section_parses_as_an_effect_without_losing_section_identity() {
    let catalog = full_dynamic_catalog();
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog.clone()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 20)
        .unwrap();

    let result = parse_effect(&mut host, &transaction, 20, "shoot zombie");
    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("shoot EffectSection must parse as a one-line Effect");
    assert_eq!(selected.matched.kind, MatchSyntaxKind::Section);
    let section = catalog
        .syntax_by_registration_id(&selected.matched.registration_id)
        .into_iter()
        .find_map(|syntax| match syntax {
            Syntax::Section(section)
                if section.common.definition_id.as_str() == selected.matched.definition_id =>
            {
                Some(section)
            }
            _ => None,
        })
        .expect("selected identity must still resolve to the source Section");
    assert!(section.effect_section);
    assert_eq!(
        section.common.element_class.as_str(),
        "ch.njol.skript.sections.EffSecShoot"
    );
    assert!(
        result
            .calls
            .iter()
            .all(|call| { call.subscription_id != "core.section-semantics" })
    );
    transaction.cancel().unwrap();
}

#[test]
fn invalid_ternary_condition_retains_parent_effect_interpretations() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 15)
        .unwrap();

    let result = parse_effect(&mut host, &transaction, 15, "send 1 if a < 5 else 2");
    let unknown = result
        .matches
        .unknown
        .expect("the invalid nested condition must keep a recoverable Effect");
    let classes = unknown
        .failures
        .candidates
        .iter()
        .filter_map(|candidate| candidate.element_class.as_ref())
        .map(|class| class.as_str())
        .collect::<Vec<_>>();
    assert!(
        classes.contains(&"org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"),
        "EffMessage must remain available beside EffDoIf: {classes:#?}"
    );
    assert!(classes.contains(&"ch.njol.skript.effects.EffDoIf"));
    let best = unknown
        .failures
        .primary()
        .expect("EffMessage must be the primary interpretation");
    let ranked = unknown
        .failures
        .candidates
        .iter()
        .map(|candidate| {
            let mut depth = 0;
            let mut trace = Some(&candidate.matched.trace);
            while let Some(current) = trace {
                depth += usize::from(current.frame.is_some());
                trace = current.cause.as_deref();
            }
            (
                candidate.element_class.as_ref().map(|class| class.as_str()),
                candidate
                    .matched
                    .trace
                    .root_cause()
                    .failure
                    .span
                    .mapped
                    .virtual_range,
                depth,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        best.element_class.as_ref().map(|class| class.as_str()),
        Some("org.skriptlang.skript.bukkit.text.elements.effects.EffMessage"),
        "ranked failures: {ranked:#?}"
    );
    assert_eq!(
        best.matched.pattern.as_deref(),
        Some("(message|send [message[s]]) %objects% [to %audiences%]")
    );
    let root = best.matched.trace.root_cause();
    assert_eq!(
        root.failure.span.mapped.virtual_range,
        TextRange::new(10, 11)
    );

    transaction.cancel().unwrap();
}

#[test]
fn expression_lists_follow_skript_conjunction_and_nesting_rules() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 17)
        .unwrap();

    for (source, conjunction, multiplicity) in [
        (
            "send 1,2,3",
            ExpressionListConjunction::And,
            Multiplicity::Multiple,
        ),
        (
            "send 1 or 2",
            ExpressionListConjunction::Or,
            Multiplicity::Single,
        ),
        (
            "send 1 and 2 or 3",
            ExpressionListConjunction::And,
            Multiplicity::Multiple,
        ),
        (
            "send 1 nor 2",
            ExpressionListConjunction::And,
            Multiplicity::Multiple,
        ),
        (
            "send 1, 2 or 3",
            ExpressionListConjunction::Or,
            Multiplicity::Single,
        ),
        (
            "send (1 and 2) or 3",
            ExpressionListConjunction::Or,
            Multiplicity::Multiple,
        ),
    ] {
        let result = parse_effect(&mut host, &transaction, 17, source);
        let selected = result
            .matches
            .selected
            .unwrap_or_else(|| panic!("{source:?} must parse as an Expression list"));
        let expression = selected.expressions().next().expect("Expression capture");
        assert_eq!(expression.kind, ExpressionNodeKind::List { conjunction });
        assert_eq!(expression.multiplicity, Some(multiplicity));
        assert!(expression.children.len() >= 2);
    }

    let source = "send spherical vector radius 1, yaw 45, pitch 90 and 2";
    let selected = parse_effect(&mut host, &transaction, 17, source)
        .matches
        .selected
        .expect("a comma-bearing vector must remain one child in an outer list");
    let expression = selected.expressions().next().expect("Expression capture");
    assert_eq!(
        expression.kind,
        ExpressionNodeKind::List {
            conjunction: ExpressionListConjunction::And,
        }
    );
    assert_eq!(expression.children.len(), 2);
    assert_eq!(
        expression.children[0].span.local_range.slice(source),
        Some("spherical vector radius 1, yaw 45, pitch 90")
    );

    let numeric = parse_effect(&mut host, &transaction, 17, "send 1, 2.5")
        .matches
        .selected
        .expect("numeric list must parse");
    assert_eq!(
        numeric
            .expressions()
            .next()
            .expect("numeric Expression")
            .return_type
            .as_ref()
            .map(|ty| ty.as_str()),
        Some("java.lang.Number")
    );

    assert!(
        parse_effect(&mut host, &transaction, 17, "send 1,and 2")
            .matches
            .selected
            .is_none(),
        "comma-adjacent `and` is part of the next piece, not the delimiter"
    );

    let invalid_utf8_child = parse_effect(&mut host, &transaction, 17, "send 1 and あ")
        .matches
        .unknown
        .expect("the invalid UTF-8 list child must retain its failure");
    assert_eq!(
        invalid_utf8_child
            .failures
            .primary()
            .expect("EffMessage remains recognizable")
            .matched
            .trace
            .root_cause()
            .failure
            .span
            .mapped
            .virtual_range,
        TextRange::new(11, 14)
    );

    for source in ["send location(1,2,3)", "send \"a,b\""] {
        let result = parse_effect(&mut host, &transaction, 17, source);
        let selected = result
            .matches
            .selected
            .unwrap_or_else(|| panic!("commas nested in {source:?} must not create a list"));
        assert!(!matches!(
            selected
                .expressions()
                .next()
                .expect("Expression capture")
                .kind,
            ExpressionNodeKind::List { .. }
        ));
    }

    transaction.cancel().unwrap();
}

#[test]
fn failed_typed_capture_covers_the_complete_expression_before_its_separator() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 18)
        .unwrap();
    let source = "teleport all player to location(1,2,3)";
    let unknown = parse_effect(&mut host, &transaction, 18, source)
        .matches
        .unknown
        .expect("invalid entity expression must retain EffTeleport");

    let failures = unknown
        .failures
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.element_class.as_ref().map(|class| class.as_str()),
                candidate
                    .matched
                    .trace
                    .root_cause()
                    .failure
                    .span
                    .mapped
                    .virtual_range,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        unknown
            .failures
            .primary()
            .expect("EffTeleport remains recognizable")
            .matched
            .trace
            .root_cause()
            .failure
            .span
            .mapped
            .virtual_range,
        TextRange::new(9, 19),
        "ranked failures: {failures:#?}"
    );
    transaction.cancel().unwrap();
}

#[test]
fn core_library_resolves_sets_only_for_supplier_backed_types() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 16)
        .unwrap();

    for source in ["send all colors", "send every color"] {
        assert!(
            parse_effect(&mut host, &transaction, 16, source)
                .matches
                .selected
                .is_some(),
            "{source:?} must use ExprSets with the ClassInfo return type"
        );
    }

    let item_alias = parse_effect(&mut host, &transaction, 16, "send all strings");
    let item_alias_candidate = item_alias
        .matches
        .selected
        .expect("strings must remain a valid ItemType alias");
    assert_eq!(
        item_alias_candidate
            .expressions()
            .next()
            .expect("item alias Expression")
            .metadata
            .get("nlaocs.core-library/literal-source")
            .map(String::as_str),
        Some("alias")
    );

    // `strings` is also a valid Minecraft ItemType alias, so use a type name
    // without an alias to exercise ExprSets' supplier rejection path.
    let invalid = parse_effect(&mut host, &transaction, 16, "send all objects");
    let best = invalid
        .matches
        .unknown
        .and_then(|unknown| unknown.failures.candidates.into_iter().next())
        .expect("the parent Effect must remain identifiable after ExprSets rejects the type");
    assert_eq!(
        best.element_class.as_ref().map(|class| class.as_str()),
        Some("org.skriptlang.skript.bukkit.text.elements.effects.EffMessage")
    );
    transaction.cancel().unwrap();
}

#[test]
fn eff_change_matches_skript_static_change_rules() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 19)
        .unwrap();

    let single_variable = parse_effect(&mut host, &transaction, 19, "set {_value} to all players");
    assert!(
        single_variable.matches.selected.is_none(),
        "a Multiple Expression must not be assigned to a single variable; selected pattern: {:?}",
        single_variable
            .matches
            .selected
            .as_ref()
            .map(|candidate| candidate.matched.pattern.as_str())
    );
    let diagnostic = single_variable
        .effects
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "core.eff-change.multiple-to-single-variable")
        .unwrap_or_else(|| {
            panic!(
                "EffChange must report the multiplicity mismatch; diagnostics={:#?}, calls={:#?}, unknown={:#?}",
                single_variable.effects.diagnostics,
                single_variable.calls,
                single_variable.matches.unknown
            )
        });
    assert_eq!(diagnostic.span.virtual_range.start, 16);
    assert_eq!(diagnostic.span.virtual_range.end, 27);
    let rejected_span = single_variable
        .matches
        .unknown
        .as_ref()
        .and_then(|unknown| unknown.failures.primary())
        .expect("the rejected EffChange candidate must remain diagnosable")
        .matched
        .trace
        .root_cause()
        .failure
        .span
        .mapped
        .virtual_range;
    assert_eq!(rejected_span, TextRange::new(16, 27));

    let multiple_variable = parse_effect(
        &mut host,
        &transaction,
        19,
        "set {_values::*} to all players",
    );
    assert!(
        multiple_variable.matches.selected.is_some(),
        "a variable list must accept a Multiple Expression: {:#?}",
        multiple_variable.matches.unknown
    );

    let single_value = parse_effect(&mut host, &transaction, 19, "set {_value} to 1");
    assert!(
        single_value.matches.selected.is_some(),
        "a single value must be assignable to a single variable: {:#?}",
        single_value.matches.unknown
    );

    let valid_expression = parse_effect(&mut host, &transaction, 19, "set whitelist to true");
    assert!(
        valid_expression.matches.selected.is_some(),
        "whitelist accepts Boolean SET values: {:#?}",
        valid_expression.matches.unknown
    );

    let wrong_expression_type =
        parse_effect(&mut host, &transaction, 19, "set whitelist to \"test\"");
    assert!(wrong_expression_type.matches.selected.is_none());
    assert!(
        wrong_expression_type
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic.code == "core.eff-change.incompatible-set-type"
                    && diagnostic.message.contains("java.lang.Boolean")
            })
    );

    let valid_property = parse_effect(
        &mut host,
        &transaction,
        19,
        "set x of velocity of all players to 1",
    );
    assert!(
        valid_property.matches.selected.is_some(),
        "WXYZ must use the Property handler's SET contract and the source Expression's changeability: {:#?}",
        valid_property.matches.unknown
    );

    let wrong_property_type = parse_effect(
        &mut host,
        &transaction,
        19,
        "set x of velocity of all players to \"test\"",
    );
    assert!(wrong_property_type.matches.selected.is_none());
    assert!(
        wrong_property_type
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic.code == "core.eff-change.incompatible-set-type"
                    && diagnostic.message.contains("java.lang.Float")
            }),
        "WXYZ must reject a String SET value using Properties.json: {:#?}",
        wrong_property_type.effects.diagnostics
    );

    let unsupported_expression = parse_effect(
        &mut host,
        &transaction,
        19,
        "set dummy direct registry expression to \"test\"",
    );
    assert!(unsupported_expression.matches.selected.is_none());
    assert!(
        unsupported_expression
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "core.eff-change.unsupported-set")
    );

    let unresolved_expression =
        parse_effect(&mut host, &transaction, 19, "set maximum double value to 1");
    assert!(
        unresolved_expression.matches.selected.is_some(),
        "an unresolved SSG contract must warn instead of guessing"
    );
    assert!(
        unresolved_expression
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "core.eff-change.unresolved-change-contract")
    );

    transaction.cancel().unwrap();
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/effect.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn parse_effect(
    host: &mut ParserHost,
    transaction: &parser_wasm::state::ParseTransaction,
    revision: u64,
    text: &str,
) -> parser_wasm::host::WasmEffectParseResult {
    let source = MappedSource::identity(text);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).expect("one Simple node");
    host.parse_effect_in_parse(
        transaction,
        context(revision),
        EffectParseRequest {
            source: &source,
            node,
            context: ExpressionParseContext::default(),
        },
        EffectParserConfig::default(),
    )
    .expect("Effect pipeline must remain recoverable")
}

#[test]
fn wasm_effect_hook_replaces_metadata_and_keeps_selected_state() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 1)
        .unwrap();

    let result = parse_effect(
        &mut host,
        &transaction,
        1,
        "dummy effect registered through wrapper",
    );
    let selected = result.matches.selected.expect("Effect must be selected");
    assert_eq!(
        selected
            .metadata
            .get("test.effect-addon/wasm")
            .map(String::as_str),
        Some("replaced")
    );
    assert_eq!(
        selected
            .metadata
            .get("test.effect-addon/catalog-annotation")
            .map(String::as_str),
        Some("replace")
    );
    assert!(
        result
            .calls
            .iter()
            .any(|call| call.subscription_id == "effect.replace")
    );
    assert!(
        result
            .calls
            .iter()
            .any(|call| call.subscription_id == "effect.not-applicable")
    );
    let not_applicable_index = result
        .calls
        .iter()
        .position(|call| call.subscription_id == "effect.not-applicable")
        .expect("NotApplicable hook must run");
    let replace_index = result
        .calls
        .iter()
        .position(|call| call.subscription_id == "effect.replace")
        .expect("replace hook must run after NotApplicable");
    assert!(not_applicable_index < replace_index);
    assert!(!selected.metadata.contains_key("temporary"));
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "effect-fixture-not-applicable")
    );
    let writes = transaction.read_write_set().unwrap().writes;
    assert!(writes.iter().any(|write| write.key == "category-before"));
    assert!(writes.iter().any(|write| write.key == "replace"));
    assert!(writes.iter().all(|write| write.key != "not-applicable"));
    assert!(writes.iter().all(|write| write.key != "reject"));
    transaction.cancel().unwrap();
}

#[test]
fn wasm_effect_reject_restores_nested_expression_and_hook_state() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 2)
        .unwrap();

    let result = parse_effect(
        &mut host,
        &transaction,
        2,
        "run dummy fixture effect with \"metadata\"",
    );
    let unknown = result
        .matches
        .unknown
        .expect("rejected Effect becomes unknown");
    assert!(unknown.failures.primary().is_some() || unknown.failures.fallback.is_some());
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "effect-fixture-reject")
    );
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "effect-fixture-reject-effects")
    );
    assert!(
        result
            .effects
            .context_updates
            .iter()
            .all(|update| update.key != "reject-effects-must-be-rolled-back")
    );
    assert_eq!(transaction.state_revision().unwrap(), 0);
    assert!(transaction.read_write_set().unwrap().writes.is_empty());
    transaction.cancel().unwrap();
}

#[test]
fn wasm_effect_reject_preserves_incomplete_near_match_and_rolls_back() {
    let input = "run dummy fixture effect with \"";
    let mut baseline_host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let baseline_transaction = baseline_host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 14)
        .unwrap();
    let baseline_result = parse_effect(&mut baseline_host, &baseline_transaction, 14, input);
    let baseline_unknown = baseline_result
        .matches
        .unknown
        .expect("incomplete Effect must remain unknown");
    assert!(baseline_unknown.failures.fallback.is_some());
    assert!(baseline_unknown.failures.primary().is_some());
    let baseline_failures = baseline_unknown.failures.clone();
    baseline_transaction.cancel().unwrap();

    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 15)
        .unwrap();

    let result = parse_effect(&mut host, &transaction, 15, input);
    let unknown = result
        .matches
        .unknown
        .expect("rejected incomplete Effect must remain unknown");
    assert_eq!(unknown.failures, baseline_failures);
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "effect-fixture-reject")
    );
    assert!(
        result
            .calls
            .iter()
            .any(|call| call.subscription_id == "effect.reject")
    );
    assert_eq!(transaction.state_revision().unwrap(), 0);
    assert!(transaction.read_write_set().unwrap().writes.is_empty());
    transaction.cancel().unwrap();
}

#[test]
fn dynamic_effect_uses_the_same_end_to_end_pipeline() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(DYNAMIC_SYNTAX_ADDON)
        .expect("dynamic syntax addon must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 3)
        .unwrap();

    let result = parse_effect(&mut host, &transaction, 3, "dummy initialize \"value\"");
    let selected = result
        .matches
        .selected
        .expect("dynamic Effect must be selected");
    assert_eq!(
        selected.matched.registration_id,
        "dynamic:nlaocs.test.dynamic-syntax/initial-effect"
    );
    assert_eq!(selected.handler.as_deref(), Some("dynamic.initial-effect"));
    assert_eq!(selected.expressions().count(), 1);
    let writes = transaction.read_write_set().unwrap().writes;
    assert!(writes.iter().any(|write| write.key == "category-before"));
    assert!(writes.iter().any(|write| write.key == "category-after"));
    transaction.cancel().unwrap();
}

fn has_hook_rejection(trace: &FailureTrace, expected: &str) -> bool {
    trace.failure.reasons.iter().any(|reason| {
        matches!(reason, PatternFailureReason::HookRejected { reason } if reason.contains(expected))
    }) || trace
        .cause
        .as_deref()
        .is_some_and(|cause| has_hook_rejection(cause, expected))
}

#[test]
fn third_party_dynamic_effect_reuses_core_input_source_parser() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(DYNAMIC_SYNTAX_ADDON)
        .expect("third-party dynamic syntax addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 20)
        .unwrap();

    // This Effect is registered by the third-party component. Its optional
    // `%string%` owns capture 0 even when omitted, while `<.+>` remains capture
    // 1 and is parsed with the InputSource context supplied by that component.
    let result = parse_effect(&mut host, &transaction, 20, "dummy scoped using input");
    let selected = result.matches.selected.expect("dynamic Effect must parse");
    assert_eq!(
        selected.matched.registration_id,
        "dynamic:nlaocs.test.dynamic-syntax/scoped-effect"
    );
    assert_eq!(selected.handler.as_deref(), Some("dynamic.scoped-effect"));
    let mapping = selected
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == 1)
        .expect("the third-party mapping capture must be routed");
    assert_eq!(mapping.binding.parser_id, "host.expression");
    assert_eq!(mapping.result.status, ParsedCaptureStatus::Success);
    assert!(matches!(
        mapping.result.value,
        Some(ParsedCaptureValue::Expression(_))
    ));
    assert!(result.calls.iter().any(|call| {
        call.component_id == "nlaocs.test.dynamic-syntax"
            && call.subscription_id == "dynamic.effect"
    }));

    // Without an Effect-local InputSource context, the same `input` token is
    // not a valid standalone expression and must not become a selected Effect.
    let contextless = parse_effect(&mut host, &transaction, 20, "send input");
    assert!(
        contextless.matches.selected.is_none(),
        "contextless input must be rejected"
    );
    let unknown = contextless
        .matches
        .unknown
        .expect("contextless input must retain a diagnostic failure");
    assert!(
        unknown
            .failures
            .candidates
            .iter()
            .any(|candidate| has_hook_rejection(&candidate.matched.trace, "InputSource")),
        "contextless input must explain that its InputSource is missing: {:#?}",
        unknown.failures
    );
    transaction.cancel().unwrap();
}

#[test]
fn dynamic_size_and_parse_expressions_work_inside_effects() {
    let catalog = full_dynamic_catalog();
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 4)
        .unwrap();

    let size_text = "all offline players's size";
    let size_source = MappedSource::identity(size_text);
    let size = host
        .parse_expression_in_parse(
            &transaction,
            context(4),
            ExpressionParseRequest {
                source: &size_source,
                range: TextRange::new(0, size_text.len()),
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName("java.lang.Number".to_owned()),
                    plural: false,
                }],
                context: ExpressionParseContext::default(),
            },
            ExpressionParserConfig::default(),
        )
        .unwrap();
    assert!(
        size.matches.selected.is_some(),
        "size must resolve as Number"
    );

    let vector = parse_effect(
        &mut host,
        &transaction,
        4,
        "send new vector from yaw all offline players's size and pitch all offline players's size",
    );
    assert!(
        vector.matches.selected.is_some(),
        "vector send must parse: {:#?}",
        vector.matches.unknown
    );
    transaction.cancel().unwrap();

    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 5)
        .unwrap();
    let parsed = parse_effect(
        &mut host,
        &transaction,
        5,
        "set {_parsed} to \"42\" parsed as number",
    );
    assert!(
        parsed.matches.selected.is_some(),
        "typed ExprParse must parse: {:#?}",
        parsed.matches.unknown
    );
    transaction.cancel().unwrap();

    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 6)
        .unwrap();
    let parsed = parse_effect(
        &mut host,
        &transaction,
        6,
        "set {_parsed::*} to \"value: 42\" parsed as \"value: %number%\"",
    );
    assert!(
        parsed.matches.selected.is_some(),
        "pattern ExprParse must parse: {:#?}",
        parsed.matches.unknown
    );
    transaction.cancel().unwrap();
}

#[test]
fn input_source_effects_parse_mappings_in_their_local_context() {
    let catalog = full_dynamic_catalog();
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 7)
        .unwrap();

    for input in [
        "transform {_a::*} using input * 2",
        "transform {_clan-sizes::*} using {clans::%input%::size}",
        "transform {_list::*} with 0",
        "sort {_words::*}",
        "sort {_words::*} by length of input",
        "sort {_words::*} in descending order by length of input",
        "sort {_words::*} based on {tastiness::%input%}",
    ] {
        let result = parse_effect(&mut host, &transaction, 7, input);
        assert!(
            result.matches.selected.is_some(),
            "InputSource Effect must parse `{input}`: {:#?}",
            result.matches.unknown
        );
        if input.contains("length of input") {
            assert!(
                result.effects.diagnostics.iter().all(|diagnostic| {
                    diagnostic.code != "core.eff-sort.unresolved-mapping-multiplicity"
                }),
                "length of input must resolve to one value: {:#?}",
                result.effects.diagnostics
            );
        }
    }

    for (input, expected_type) in [
        ("sort {_words::*} by input", "java.lang.Object"),
        ("sort {_words::*} by number input", "java.lang.Number"),
        ("sort {_words::*} by input index", "java.lang.String"),
    ] {
        let result = parse_effect(&mut host, &transaction, 7, input);
        let selected = result.matches.selected.unwrap_or_else(|| {
            panic!(
                "typed InputSource mapping must parse `{input}`: {:#?}",
                result.matches.unknown
            )
        });
        let mapping = selected
            .parsed_captures
            .iter()
            .find(|capture| capture.capture_index == 1)
            .expect("the sort mapping must retain its parsed Expression");
        let Some(ParsedCaptureValue::Expression(expression)) = mapping.result.value.as_ref() else {
            panic!("the sort mapping must be an Expression");
        };
        assert_eq!(
            expression.return_type.as_ref().map(|value| value.as_str()),
            Some(expected_type),
            "unexpected return type for `{input}`"
        );
        assert_eq!(expression.multiplicity, Some(Multiplicity::Single));
    }

    for input in [
        "transform {_value} using input",
        "sort {_words::*} by {_values::*}",
        "send input",
    ] {
        let result = parse_effect(&mut host, &transaction, 7, input);
        assert!(
            result.matches.selected.is_none(),
            "invalid InputSource usage must be rejected: `{input}`"
        );
    }
    transaction.cancel().unwrap();
}

#[test]
fn nested_parenthesized_expressions_work_inside_effects() {
    let catalog = full_dynamic_catalog();
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 8)
        .unwrap();
    let input = "send (new vector from yaw (all offline players's size) and pitch (all offline players's size))";
    let result = parse_effect(&mut host, &transaction, 8, input);
    let selected = result.matches.selected.unwrap_or_else(|| {
        panic!(
            "nested parenthesized vector must parse: {:#?}",
            result.matches.unknown
        )
    });

    let grouped_vector = selected.expressions().next().expect("grouped vector");
    assert!(matches!(grouped_vector.kind, ExpressionNodeKind::Grouped));
    let vector = &grouped_vector.children[0];
    assert!(matches!(vector.kind, ExpressionNodeKind::Registered { .. }));
    assert_eq!(vector.children.len(), 2);
    assert!(
        vector
            .children
            .iter()
            .all(|child| matches!(child.kind, ExpressionNodeKind::Grouped))
    );
    transaction.cancel().unwrap();
}

fn full_dynamic_catalog_with_vector_overload() -> Arc<Catalog> {
    let source = full_dynamic_catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let mut syntaxes = source.syntaxes().to_vec();
    let mut vector = source
        .functions_named("vector")
        .into_iter()
        .next()
        .expect("fixture has vector(n)")
        .clone();
    vector.registration_order = syntaxes.len();
    vector.registration_id = RegistrationId("test:function:vector:x-y-z".to_owned());
    vector.parameters = ["x", "y", "z"]
        .into_iter()
        .map(|name| FunctionParameter {
            name: name.to_owned(),
            parameter_type: ClassName("java.lang.Number".to_owned()),
            modifiers: Vec::new(),
            single: true,
        })
        .collect();
    syntaxes.push(Syntax::Function(vector));

    Arc::new(
        Catalog::new(CatalogParts {
            syntaxes,
            converters: source.converters().to_vec(),
            comparators: source.comparators().to_vec(),
            event_values: source.event_values().to_vec(),
            properties: source.properties().to_vec(),
            operators: source.operators().to_vec(),
            operations: source.operations().clone(),
            differences: source.differences().to_vec(),
            classes: source.classes().to_vec(),
            aliases: source.aliases().clone(),
            plural_rules: source.plural_rules().clone(),
            language: source
                .language_entries()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
        })
        .with_unchecked_source(source_view),
    )
}

#[test]
fn arithmetic_uses_snapshot_operations_and_skript_precedence() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 10)
        .unwrap();

    let precedence = parse_effect(&mut host, &transaction, 10, "return 1 + 2 * 3")
        .matches
        .selected
        .expect("typed arithmetic must parse");
    let root = precedence.expressions().next().expect("arithmetic root");
    assert!(matches!(
        root.kind,
        ExpressionNodeKind::Arithmetic { ref operator, .. } if operator == "+"
    ));
    assert!(matches!(
        root.children[1].kind,
        ExpressionNodeKind::Arithmetic { ref operator, .. } if operator == "*"
    ));

    let left_associative = parse_effect(&mut host, &transaction, 10, "return 1 - 2 - 3")
        .matches
        .selected
        .expect("same-priority arithmetic must parse");
    let root = left_associative
        .expressions()
        .next()
        .expect("left-associative root");
    assert!(matches!(
        root.kind,
        ExpressionNodeKind::Arithmetic { ref operator, .. } if operator == "-"
    ));
    assert!(matches!(
        root.children[0].kind,
        ExpressionNodeKind::Arithmetic { ref operator, .. } if operator == "-"
    ));

    let unary = parse_effect(&mut host, &transaction, 10, "return 1 * -1")
        .matches
        .selected
        .expect("negative literals must remain operands");
    assert!(matches!(
        unary.expressions().next().expect("unary root").kind,
        ExpressionNodeKind::Arithmetic { ref operator, .. } if operator == "*"
    ));
    transaction.cancel().unwrap();
}

#[test]
fn overloaded_function_accepts_multiple_arithmetic_arguments() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog_with_vector_overload()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 11)
        .unwrap();
    let input = "return vector(all offline players's size * -1, all offline players's size * -1, all offline players's size * -1)";
    let selected = parse_effect(&mut host, &transaction, 11, input)
        .matches
        .selected
        .expect("vector(x, y, z) must accept arithmetic arguments");

    let vector = selected.expressions().next().expect("vector Expression");
    assert!(matches!(vector.kind, ExpressionNodeKind::Function { .. }));
    assert_eq!(vector.function.as_ref().unwrap().arguments.len(), 3);
    assert_eq!(vector.children.len(), 3);
    assert!(vector.children.iter().all(|child| matches!(
        child.kind,
        ExpressionNodeKind::Arithmetic { ref operator, .. } if operator == "*"
    )));
    transaction.cancel().unwrap();
}

#[test]
fn property_expression_return_type_flows_into_function_arguments() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 12)
        .unwrap();
    let input = "teleport \"nlaocs\" parsed as player to location(\"nlaocs\" parsed as player's location's x-coord, 1, 1)";
    let result = parse_effect(&mut host, &transaction, 12, input);
    let selected = result.matches.selected.unwrap_or_else(|| {
        let failures = result.matches.unknown.as_ref().map(|unknown| {
            unknown
                .failures
                .candidates
                .iter()
                .map(|candidate| {
                    let root = candidate.matched.trace.root_cause();
                    (
                        candidate.element_class.as_ref().map(ClassName::as_str),
                        candidate.matched.pattern.as_deref(),
                        root.failure.span.mapped.virtual_range,
                        &root.failure.reasons,
                    )
                })
                .collect::<Vec<_>>()
        });
        panic!("Location coordinate must satisfy the Number function parameter: {failures:#?}")
    });

    let location = selected
        .expressions()
        .find(|expression| {
            expression
                .function
                .as_ref()
                .is_some_and(|function| function.name == "location")
        })
        .expect("teleport Effect contains the location Function");
    let x = &location.children[0];
    assert!(matches!(x.kind, ExpressionNodeKind::Registered { .. }));
    assert_eq!(
        x.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Double")
    );
    assert_eq!(
        x.metadata
            .get("nlaocs.core-library/wxyz-axis")
            .map(String::as_str),
        Some("x")
    );
    transaction.cancel().unwrap();
}

#[test]
fn property_expression_prefers_the_closest_source_handler() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 13)
        .unwrap();
    let selected = parse_effect(
        &mut host,
        &transaction,
        13,
        "send \"nlaocs\" parsed as player's name",
    )
    .matches
    .selected
    .expect("the Player name property must parse");

    let name = selected.expressions().next().expect("name Expression");
    assert_eq!(
        name.metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("name-property")
    );
    assert_eq!(
        name.return_type.as_ref().map(ClassName::as_str),
        Some("net.kyori.adventure.text.Component")
    );
    transaction.cancel().unwrap();
}

#[test]
fn conditional_effect_retains_its_nested_effect_and_condition() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_dynamic_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 9)
        .unwrap();
    let result = parse_effect(
        &mut host,
        &transaction,
        9,
        "dummy effect registered through wrapper if dummy fixture condition",
    );
    let selected = result.matches.selected.unwrap_or_else(|| {
        panic!(
            "conditional Effect must parse: {:#?}",
            result.matches.unknown
        )
    });

    assert_eq!(selected.effects().count(), 1);
    assert_eq!(selected.conditions().count(), 1);
    assert!(result.calls.iter().any(|call| {
        call.component_id == "nlaocs.core-library"
            && call.subscription_id == "core.effect-semantics"
    }));
    transaction.cancel().unwrap();
}

#[test]
fn dynamic_player_expressions_are_valid_audiences() {
    let catalog = full_dynamic_catalog();
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 7)
        .unwrap();

    for input in [
        "send 1 to all players",
        "send 1 to \"nlaocs\" parsed as player",
        "send new vector from yaw all offline players's size and pitch all offline players's size to all players",
    ] {
        let result = parse_effect(&mut host, &transaction, 7, input);
        let selected = result.matches.selected.unwrap_or_else(|| {
            panic!(
                "dynamic Player must parse as Audience: {:#?}",
                result.matches.unknown
            )
        });
        let recipient = selected
            .expressions()
            .last()
            .expect("message Effect has a recipient");
        assert_eq!(
            recipient.return_type.as_ref().map(ClassName::as_str),
            Some("org.bukkit.entity.Player"),
            "{input:?} must keep the narrowed dynamic return type: {recipient:#?}"
        );
    }
    transaction.cancel().unwrap();
}
