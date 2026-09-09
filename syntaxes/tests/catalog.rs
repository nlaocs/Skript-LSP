use std::collections::BTreeMap;
use syntax_pattern_parser::syntax::PluralRules;
use syntaxes::{
    Addon, AliasItem, AliasRegistry, AliasTarget, Catalog, CatalogParts, CatalogSource, Class,
    ClassKind, ClassMethod, ClassName, Converter, DefinitionId, Difference, Documentation,
    EventValue, Function, FunctionParameter, Noun, RegisteredTypeParserPattern, RegistrationId,
    Syntax, Type, TypeCodeName, TypeLiteral, TypeLiteralSource,
};

fn id(value: &str) -> RegistrationId {
    RegistrationId(value.to_owned())
}

fn class_name(value: &str) -> ClassName {
    ClassName(value.to_owned())
}

fn addon() -> Addon {
    Addon {
        name: "TestAddon".to_owned(),
        version: "1.0.0".to_owned(),
    }
}

fn class(name: &str, super_class: Option<&str>, interfaces: &[&str]) -> Class {
    Class {
        name: class_name(name),
        binary_name: name.to_owned(),
        kind: ClassKind::Class,
        super_class: super_class.map(class_name),
        interfaces: interfaces.iter().map(|value| class_name(value)).collect(),
        component_type: None,
        container_element_type: None,
        methods: None,
        provider: None,
    }
}

fn event_value(
    event_class: &str,
    value_class: &str,
    resolution_order: usize,
    excludes: &[&str],
) -> EventValue {
    EventValue {
        event_class: class_name(event_class),
        value_class: class_name(value_class),
        time: 0,
        exclude_error_message: None,
        excludes: (!excludes.is_empty())
            .then(|| excludes.iter().map(|value| class_name(value)).collect()),
        resolution_order,
        registration_order: Some(resolution_order),
        patterns: Some(Vec::new()),
        accepted_changers: Some(BTreeMap::new()),
        context_dependent: None,
        has_custom_input_validator: None,
        has_custom_event_validator: None,
        addon: addon(),
        registration_id: id(&format!("event-value-{resolution_order}")),
    }
}

fn function(
    name: &str,
    registration_id: &str,
    registration_order: usize,
    parameter_type: Option<&str>,
) -> Syntax {
    Syntax::Function(Function {
        registration_order,
        name: name.to_owned(),
        documentation: Documentation {
            name: Some(name.to_owned()),
            ..Documentation::default()
        },
        return_type: None,
        return_type_is_single: true,
        parameters: parameter_type
            .map(|parameter_type| FunctionParameter {
                name: "value".to_owned(),
                parameter_type: class_name(parameter_type),
                modifiers: Vec::new(),
                single: true,
            })
            .into_iter()
            .collect(),
        addon: addon(),
        definition_id: DefinitionId(format!("definition-{registration_order}")),
        registration_id: id(registration_id),
    })
}

fn type_syntax(code_name: &str, assignable_to: &[&str], order: usize) -> Syntax {
    Syntax::Type(Type {
        source_index: order,
        type_parse_order: order,
        documentation: Documentation::default(),
        addon: addon(),
        definition_id: DefinitionId(format!("type-definition-{order}")),
        registration_id: id(&format!("type-{code_name}")),
        has_docs: false,
        changer: None,
        original_class: class_name(&format!("test.{code_name}")),
        class_type: ClassKind::Class,
        code_name: TypeCodeName(code_name.to_owned()),
        super_class: None,
        interfaces: Vec::new(),
        assignable_to: assignable_to
            .iter()
            .map(|value| TypeCodeName((*value).to_owned()))
            .collect(),
        user_input_patterns: Vec::new(),
        noun: Noun {
            key: code_name.to_owned(),
            value: None,
            singular: code_name.to_owned(),
            plural: format!("{code_name}s"),
            gender: 0,
            gender_id: "none".to_owned(),
        },
        serialize_as: None,
        usage: Vec::new(),
        enum_values: Vec::new(),
        parser_patterns: Vec::new(),
        registered_parser_patterns: Vec::new(),
        literal_values: Vec::new(),
        type_literals: Vec::new(),
        parser_class: None,
        parse_contexts: Vec::new(),
        default_expression: None,
        has_parser: false,
        has_serializer: false,
        has_supplier: false,
        properties: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
    })
}

fn parts() -> CatalogParts {
    CatalogParts {
        syntaxes: Vec::new(),
        converters: Vec::new(),
        comparators: Vec::new(),
        event_values: Vec::new(),
        properties: Vec::new(),
        operators: Vec::new(),
        operations: BTreeMap::new(),
        differences: Vec::new(),
        classes: Vec::new(),
        aliases: AliasRegistry {
            aliases: BTreeMap::new(),
            targets: Vec::new(),
        },
        plural_rules: PluralRules::from_json(
            r#"{
                "algorithm":"singular-aware",
                "pluralOverrideSupported":false,
                "rules":[{
                    "ruleOrder":0,
                    "singular":"",
                    "plural":"s",
                    "completeWord":false,
                    "origin":"built-in",
                    "addon":{"name":"Skript","version":"test"}
                }]
            }"#,
        )
        .unwrap(),
        language: BTreeMap::new(),
    }
}

#[test]
fn class_info_lookup_is_exact_and_preserves_the_first_registration() {
    let mut parts = parts();
    let first = type_syntax("parent", &[], 0);
    let Syntax::Type(mut duplicate) = type_syntax("duplicate", &[], 1) else {
        unreachable!()
    };
    duplicate.original_class = class_name("test.parent");
    parts.syntaxes = vec![
        first,
        Syntax::Type(duplicate),
        type_syntax("child", &["parent"], 2),
    ];
    let catalog = Catalog::new(parts);
    assert_eq!(
        catalog
            .type_by_class_name("test.parent")
            .unwrap()
            .code_name
            .as_str(),
        "parent"
    );
    assert_eq!(
        catalog
            .type_by_class_name("test.child")
            .unwrap()
            .code_name
            .as_str(),
        "child"
    );
    assert!(catalog.type_by_class_name("test.missing").is_none());
    assert!(catalog.type_by_class_name("TEST.parent").is_none());
}

#[test]
fn language_values_preserve_exact_keys_and_deterministic_iteration() {
    let mut parts = parts();
    parts.language = BTreeMap::from([
        ("message.empty".to_owned(), String::new()),
        ("message.send".to_owned(), "Send".to_owned()),
    ]);
    let catalog = Catalog::new(parts);

    assert_eq!(catalog.language_value("message.send"), Some("Send"));
    assert_eq!(catalog.language_value("message.empty"), Some(""));
    assert_eq!(catalog.language_value("MESSAGE.SEND"), None);
    assert_eq!(
        catalog.language_entries().collect::<Vec<_>>(),
        [("message.empty", ""), ("message.send", "Send")]
    );
}

#[test]
fn registered_type_patterns_are_preparsed_and_matched_in_registration_order() {
    let Syntax::Type(mut entity_data) = type_syntax("entitydata", &[], 0) else {
        unreachable!();
    };
    entity_data.has_parser = true;
    entity_data.registered_parser_patterns = vec![
        RegisteredTypeParserPattern {
            pattern: "charged creepers".to_owned(),
            registration_index: 3,
            pattern_index: 0,
            source_code_name: Some("specific charged creepers".to_owned()),
            data_class: class_name("test.SpecificCreeperData"),
            represented_class: class_name("test.SpecificCreeper"),
        },
        RegisteredTypeParserPattern {
            pattern: "enderman [holding %-itemtype%]".to_owned(),
            registration_index: 2,
            pattern_index: 0,
            source_code_name: Some("enderman".to_owned()),
            data_class: class_name("test.EndermanData"),
            represented_class: class_name("test.Enderman"),
        },
        RegisteredTypeParserPattern {
            pattern: "(powered|charged) creeper[plural:s]".to_owned(),
            registration_index: 1,
            pattern_index: 0,
            source_code_name: Some("powered creeper".to_owned()),
            data_class: class_name("test.PoweredCreeperData"),
            represented_class: class_name("test.Creeper"),
        },
    ];
    let registration_id = entity_data.registration_id.as_str().to_owned();
    let mut parts = parts();
    parts.syntaxes.push(Syntax::Type(entity_data));
    let catalog = Catalog::new(parts);

    let matched = catalog
        .registered_type_pattern_match(&registration_id, "charged creepers")
        .expect("the finite registered pattern must match");
    assert_eq!(matched.registration.registration_index, 1);
    assert_eq!(
        matched.registration.source_code_name.as_deref(),
        Some("powered creeper")
    );
    assert_eq!(
        matched.registration.represented_class.as_str(),
        "test.Creeper"
    );
    assert!(matched.tags.iter().any(|tag| tag == "plural"));

    assert!(
        catalog
            .registered_type_pattern_match(&registration_id, "enderman holding stone")
            .is_none(),
        "typed placeholders remain the WASM provider's responsibility"
    );
}

#[test]
fn class_assignability_follows_superclasses_and_interfaces() {
    let mut parts = parts();
    parts.classes = vec![
        class("test.Root", None, &[]),
        class("test.Named", None, &[]),
        class("test.Middle", Some("test.Root"), &["test.Named"]),
        class("test.Leaf", Some("test.Middle"), &[]),
    ];
    let catalog = Catalog::new(parts);

    assert!(catalog.is_class_assignable("test.Leaf", "test.Leaf"));
    assert!(catalog.is_class_assignable("test.Leaf", "test.Root"));
    assert!(catalog.is_class_assignable("test.Leaf", "test.Named"));
    assert!(!catalog.is_class_assignable("test.Root", "test.Leaf"));
    assert!(!catalog.is_class_assignable("test.Missing", "test.Root"));
    assert_eq!(
        catalog.hierarchy_distance("test.Leaf", "test.Leaf"),
        Some(0)
    );
    assert_eq!(
        catalog.hierarchy_distance("test.Root", "test.Leaf"),
        Some(2)
    );
    assert_eq!(
        catalog.hierarchy_distance("test.Named", "test.Leaf"),
        Some(3)
    );
    assert_eq!(catalog.hierarchy_distance("test.Leaf", "test.Root"), None);
}

#[test]
fn difference_options_prefer_exact_then_nearest_registered_input() {
    let mut parts = parts();
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        class("java.lang.Number", Some("java.lang.Object"), &[]),
        class("java.lang.Long", Some("java.lang.Number"), &[]),
    ];
    parts.differences = vec![
        Difference {
            input_type: class_name("java.lang.Object"),
            return_type: class_name("java.lang.Object"),
            registration_order: 0,
            addon: addon(),
            registration_id: id("object-difference"),
        },
        Difference {
            input_type: class_name("java.lang.Number"),
            return_type: class_name("java.lang.Number"),
            registration_order: 1,
            addon: addon(),
            registration_id: id("number-difference"),
        },
        Difference {
            input_type: class_name("java.lang.Long"),
            return_type: class_name("java.lang.Long"),
            registration_order: 2,
            addon: addon(),
            registration_id: id("long-difference"),
        },
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog
            .difference_options_for_type("java.lang.Long")
            .into_iter()
            .map(|difference| difference.registration_id.as_str())
            .collect::<Vec<_>>(),
        ["long-difference", "number-difference", "object-difference"]
    );
}

#[test]
fn converter_compatibility_includes_assignable_inputs_and_dynamic_objects() {
    let mut parts = parts();
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        class("java.lang.Number", Some("java.lang.Object"), &[]),
        class("java.lang.Long", Some("java.lang.Number"), &[]),
        class("java.lang.Float", Some("java.lang.Number"), &[]),
    ];
    parts.converters = vec![Converter {
        from: class_name("java.lang.Number"),
        to: class_name("java.lang.Float"),
        flags: 0,
        registration_order: 0,
        addon: addon(),
        registration_id: id("number-to-float"),
    }];
    let catalog = Catalog::new(parts);

    assert!(catalog.can_convert("java.lang.Long", "java.lang.Number"));
    assert!(catalog.can_convert("java.lang.Long", "java.lang.Float"));
    assert!(catalog.can_convert("java.lang.Object", "java.lang.Float"));
    assert!(!catalog.can_convert("java.lang.Float", "java.lang.Long"));
}

#[test]
fn common_assignable_class_prefers_the_nearest_shared_parent() {
    let mut parts = parts();
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        class("test.Number", Some("java.lang.Object"), &[]),
        class("test.Long", Some("test.Number"), &[]),
        class("test.Double", Some("test.Number"), &[]),
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.common_assignable_class("test.Long", "test.Double"),
        Some(class_name("test.Number"))
    );
}

#[test]
fn class_assignability_terminates_on_cycles() {
    let mut parts = parts();
    parts.classes = vec![
        class("test.A", Some("test.B"), &[]),
        class("test.B", Some("test.A"), &[]),
        class("test.Other", None, &[]),
    ];
    let catalog = Catalog::new(parts);

    assert!(catalog.is_class_assignable("test.A", "test.B"));
    assert!(catalog.is_class_assignable("test.B", "test.A"));
    assert!(!catalog.is_class_assignable("test.A", "test.Other"));
}

#[test]
fn known_java_reference_types_are_assignable_to_object() {
    let mut interface = class("test.Named", None, &[]);
    interface.kind = ClassKind::Interface;
    let mut array = class("test.Named[]", None, &[]);
    array.kind = ClassKind::Array;
    let mut primitive = class("int", None, &[]);
    primitive.kind = ClassKind::Primitive;
    let mut parts = parts();
    parts.classes = vec![interface, array, primitive];
    let catalog = Catalog::new(parts);

    assert!(catalog.is_class_assignable("test.Named", "java.lang.Object"));
    assert!(catalog.is_class_assignable("test.Named[]", "java.lang.Object"));
    assert!(!catalog.is_class_assignable("int", "java.lang.Object"));
    assert!(!catalog.is_class_assignable("test.Missing", "java.lang.Object"));
}

#[test]
fn catalog_source_preserves_documents_and_indexes_duplicate_ids() {
    let manifest = br#"{"snapshotId":"snapshot-id","futureManifestField":true}"#;
    let effects = br#"[
        {
            "registrationId":"shared-registration",
            "definitionId":"shared-definition",
            "futureField":{"enabled":true}
        },
        {
            "registrationId":"shared-registration",
            "definitionId":"shared-definition",
            "futureField":{"enabled":false}
        },
        {
            "registrationId":"other-registration",
            "definitionId":"shared-definition"
        }
    ]"#;
    let source = CatalogSource::from_json_documents(
        "ssg",
        3,
        "snapshot-id",
        BTreeMap::from([
            ("Effects.json".to_owned(), effects.to_vec()),
            ("Manifest.json".to_owned(), manifest.to_vec()),
        ]),
    )
    .expect("valid source documents must be retained");

    assert_eq!(source.document("Effects.json"), Some(effects.as_slice()));
    assert_eq!(
        source.document_names().collect::<Vec<_>>(),
        ["Effects.json", "Manifest.json"]
    );

    let registrations = source.records_by_registration_id("shared-registration");
    assert_eq!(
        registrations
            .iter()
            .map(|record| record.index)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(
        registrations
            .iter()
            .all(|record| record.document == "Effects.json")
    );

    let definitions = source.records_by_definition_id("shared-definition");
    assert_eq!(
        definitions
            .iter()
            .map(|record| record.index)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );

    let first_record: serde_json::Value =
        serde_json::from_slice(registrations[0].json.as_ref()).unwrap();
    assert_eq!(first_record["futureField"]["enabled"], true);

    let changed_manifest = CatalogSource::from_json_documents(
        "ssg",
        3,
        "snapshot-id",
        BTreeMap::from([
            ("Effects.json".to_owned(), effects.to_vec()),
            (
                "Manifest.json".to_owned(),
                br#"{"snapshotId":"snapshot-id","futureManifestField":false}"#.to_vec(),
            ),
        ]),
    )
    .unwrap();
    assert_eq!(source.snapshot_id, changed_manifest.snapshot_id);
    assert_ne!(source.source_digest, changed_manifest.source_digest);
}

#[test]
fn type_assignability_uses_declared_ssg_relationships() {
    let mut parts = parts();
    parts.syntaxes = vec![
        type_syntax("player", &["entity", "object"], 0),
        type_syntax("entity", &["object"], 1),
        type_syntax("object", &[], 2),
    ];
    let catalog = Catalog::new(parts);

    assert!(catalog.is_type_assignable("player", "player"));
    assert!(catalog.is_type_assignable("player", "entity"));
    assert!(catalog.is_type_assignable("player", "object"));
    assert!(!catalog.is_type_assignable("entity", "player"));
    assert!(!catalog.is_type_assignable("missing", "object"));
}

#[test]
fn event_values_inherit_filter_exclusions_and_follow_resolution_order() {
    let mut parts = parts();
    parts.classes = vec![
        class("test.ParentEvent", None, &[]),
        class("test.ChildEvent", Some("test.ParentEvent"), &[]),
        class("test.OtherEvent", None, &[]),
        class("test.FirstValue", None, &[]),
        class("test.SecondValue", None, &[]),
        class("test.ExcludedValue", None, &[]),
        class("test.OtherValue", None, &[]),
    ];
    parts.event_values = vec![
        event_value("test.ChildEvent", "test.SecondValue", 2, &[]),
        event_value("test.ParentEvent", "test.FirstValue", 1, &[]),
        event_value(
            "test.ParentEvent",
            "test.ExcludedValue",
            0,
            &["test.ChildEvent"],
        ),
        event_value("test.OtherEvent", "test.OtherValue", 3, &[]),
    ];
    let catalog = Catalog::new(parts);

    let values = catalog
        .event_values_for("test.ChildEvent")
        .into_iter()
        .map(|value| value.value_class.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, ["test.FirstValue", "test.SecondValue"]);
}

#[test]
fn event_value_inheritance_terminates_on_class_cycles() {
    let mut parts = parts();
    parts.classes = vec![
        class("test.A", Some("test.B"), &[]),
        class("test.B", Some("test.A"), &[]),
        class("test.Value", None, &[]),
    ];
    parts.event_values = vec![
        event_value("test.A", "test.Value", 0, &[]),
        event_value("test.B", "test.Value", 1, &[]),
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(catalog.event_values_for("test.A").len(), 2);
}

#[test]
fn indexes_functions_converters_registration_ids_and_aliases() {
    let mut parts = parts();
    parts.syntaxes = vec![
        function("lookup", "function-one", 0, None),
        function("lookup", "function-two", 1, Some("test.Argument")),
    ];
    parts.converters = vec![
        Converter {
            from: class_name("test.Source"),
            to: class_name("test.First"),
            flags: 0,
            registration_order: 0,
            addon: addon(),
            registration_id: id("converter-one"),
        },
        Converter {
            from: class_name("test.Source"),
            to: class_name("test.Second"),
            flags: 0,
            registration_order: 1,
            addon: addon(),
            registration_id: id("converter-two"),
        },
    ];
    parts.aliases.aliases.insert("example".to_owned(), 0);
    parts.aliases.targets.push(AliasTarget {
        amount: 1,
        all: false,
        types: vec![AliasItem {
            material: "STONE".to_owned(),
            minecraft_id: Some("minecraft:stone".to_owned()),
            durability: 0,
            plain: true,
            alias: false,
            block_values: None,
            item_meta: None,
        }],
    });
    let catalog = Catalog::new(parts);

    assert_eq!(catalog.functions_named("lookup").len(), 2);
    assert_eq!(catalog.functions_named("missing").len(), 0);
    assert_eq!(catalog.syntax_by_registration_id("function-one").len(), 1);
    assert_eq!(catalog.syntax_by_registration_id("missing").len(), 0);
    assert_eq!(catalog.converters_from("test.Source").len(), 2);
    assert_eq!(catalog.converters_to("test.Second").len(), 1);
    assert_eq!(catalog.alias("example").unwrap().types[0].material, "STONE");
    assert!(catalog.alias("missing").is_none());
}

#[test]
fn common_assignable_class_prefers_a_shared_interface_over_object() {
    let mut interface = class("test.Shared", None, &[]);
    interface.kind = ClassKind::Interface;
    let mut parts = parts();
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        interface,
        class("test.Left", Some("java.lang.Object"), &["test.Shared"]),
        class("test.Right", Some("java.lang.Object"), &["test.Shared"]),
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.common_assignable_class("test.Left", "test.Right"),
        Some(class_name("test.Shared"))
    );
}

#[test]
fn common_assignable_classes_folds_more_than_two_types_in_order() {
    let mut interface = class("test.Shared", None, &[]);
    interface.kind = ClassKind::Interface;
    let mut parts = parts();
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        interface,
        class("test.A", Some("java.lang.Object"), &["test.Shared"]),
        class("test.B", Some("java.lang.Object"), &["test.Shared"]),
        class("test.C", Some("java.lang.Object"), &["test.Shared"]),
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.common_assignable_classes(&[
            class_name("test.A"),
            class_name("test.B"),
            class_name("test.C"),
        ]),
        Some(class_name("test.Shared"))
    );
}

#[test]
fn common_assignable_class_does_not_expose_cloneable() {
    let mut cloneable = class("java.lang.Cloneable", None, &[]);
    cloneable.kind = ClassKind::Interface;
    let mut left = class(
        "test.LeftArray",
        Some("java.lang.Object"),
        &["java.lang.Cloneable"],
    );
    left.kind = ClassKind::Array;
    let mut right = class(
        "test.RightArray",
        Some("java.lang.Object"),
        &["java.lang.Cloneable"],
    );
    right.kind = ClassKind::Array;
    let mut parts = parts();
    parts.classes = vec![class("java.lang.Object", None, &[]), cloneable, left, right];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.common_assignable_class("test.LeftArray", "test.RightArray"),
        Some(class_name("java.lang.Object"))
    );
}

#[test]
fn common_skript_class_normalizes_an_unregistered_interface() {
    let mut interface = class("test.Shared", None, &[]);
    interface.kind = ClassKind::Interface;
    let mut object_syntax = type_syntax("object", &[], 0);
    let Syntax::Type(object_type) = &mut object_syntax else {
        unreachable!()
    };
    object_type.original_class = class_name("java.lang.Object");
    let mut parts = parts();
    parts.syntaxes = vec![object_syntax];
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        interface,
        class("test.Left", Some("java.lang.Object"), &["test.Shared"]),
        class("test.Right", Some("java.lang.Object"), &["test.Shared"]),
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.common_skript_class(&[class_name("test.Left"), class_name("test.Right"),]),
        Some(class_name("java.lang.Object"))
    );
}

#[test]
fn common_skript_class_uses_skript_type_parse_order() {
    let mut base = class("test.Base", None, &[]);
    base.kind = ClassKind::Interface;
    let mut shared = class("test.Shared", None, &["test.Base"]);
    shared.kind = ClassKind::Interface;

    let mut object_syntax = type_syntax("object", &[], 10);
    let Syntax::Type(object_type) = &mut object_syntax else {
        unreachable!()
    };
    object_type.original_class = class_name("java.lang.Object");

    let mut base_syntax = type_syntax("base", &[], 1);
    let Syntax::Type(base_type) = &mut base_syntax else {
        unreachable!()
    };
    base_type.original_class = class_name("test.Base");

    let mut parts = parts();
    parts.syntaxes = vec![object_syntax, base_syntax];
    parts.classes = vec![
        class("java.lang.Object", None, &[]),
        base,
        shared,
        class("test.Left", Some("java.lang.Object"), &["test.Shared"]),
        class("test.Right", Some("java.lang.Object"), &["test.Shared"]),
    ];
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.common_assignable_class("test.Left", "test.Right"),
        Some(class_name("test.Shared")),
        "the raw common interface is intentionally not registered as a Skript Type"
    );
    assert_eq!(
        catalog.common_skript_class(&[class_name("test.Left"), class_name("test.Right"),]),
        Some(class_name("test.Base"))
    );
}

#[test]
fn indexes_only_parseable_type_literals_in_type_order() {
    let mut parts = parts();
    let mut later = match type_syntax("later", &[], 20) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    later.has_parser = true;
    later.literal_values = vec!["shared".to_owned()];
    let mut earlier = match type_syntax("earlier", &[], 10) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    earlier.has_parser = true;
    earlier.parser_patterns = vec!["Shared".to_owned()];
    let mut enum_without_parser = match type_syntax("rawenum", &[], 0) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    enum_without_parser.enum_values = vec!["shared".to_owned()];
    parts.syntaxes = vec![
        Syntax::Type(later),
        Syntax::Type(earlier),
        Syntax::Type(enum_without_parser),
    ];
    let catalog = Catalog::new(parts);

    let matches = catalog
        .type_literals(" SHARED ")
        .map(|value| value.code_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(matches, ["earlier", "later"]);
}

#[test]
fn item_aliases_are_type_literals_and_alias_lookup_is_case_insensitive() {
    let mut parts = parts();
    let mut item_type = match type_syntax("itemtype", &[], 0) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    item_type.has_parser = true;
    parts.syntaxes = vec![Syntax::Type(item_type)];
    parts.aliases.aliases.insert("stone".to_owned(), 0);
    parts.aliases.targets.push(AliasTarget {
        amount: 1,
        all: false,
        types: Vec::new(),
    });
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog
            .type_literals("STONE")
            .next()
            .map(|value| value.code_name.as_str()),
        Some("itemtype")
    );
    assert!(catalog.alias("STONE").is_some());
}

#[test]
fn detailed_type_literal_matches_include_supplier_plural_metadata() {
    let mut parts = parts();
    let mut entity = match type_syntax("entity", &[], 0) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    entity.has_parser = true;
    entity.has_supplier = true;
    entity.literal_values = vec!["zombie".to_owned()];
    entity.type_literals = vec![TypeLiteral {
        text: "zombie".to_owned(),
        plural_text: Some("zombies".to_owned()),
        variable_name: Some("entitydata:zombie".to_owned()),
        debug_text: None,
        value_class: class_name("ch.njol.skript.entity.SimpleEntityData"),
        represented_class: Some(class_name("org.bukkit.entity.Zombie")),
        enum_constant: None,
    }];
    parts.syntaxes = vec![Syntax::Type(entity)];
    let catalog = Catalog::new(parts);

    let singular = catalog.type_literal_matches("zombie").collect::<Vec<_>>();
    assert_eq!(singular.len(), 1);
    assert_eq!(singular[0].type_info.code_name.as_str(), "entity");
    assert_eq!(singular[0].canonical_value, "zombie");
    assert!(!singular[0].plural);
    assert_eq!(singular[0].source, TypeLiteralSource::Supplier);
    assert_eq!(
        singular[0]
            .literal
            .and_then(|literal| literal.represented_class.as_ref())
            .map(|class| class.as_str()),
        Some("org.bukkit.entity.Zombie")
    );

    let plural = catalog.type_literal_matches("zombies").collect::<Vec<_>>();
    assert_eq!(plural.len(), 1);
    assert_eq!(plural[0].canonical_value, "zombie");
    assert!(plural[0].plural);
    assert_eq!(plural[0].source, TypeLiteralSource::Supplier);
}

#[test]
fn aliases_get_plural_matches_but_enum_constants_do_not() {
    let mut parts = parts();
    let mut item_type = match type_syntax("itemtype", &[], 20) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    item_type.has_parser = true;
    parts.syntaxes = vec![Syntax::Type(item_type)];
    parts.aliases.aliases.insert("zombie".to_owned(), 0);
    parts.aliases.targets.push(AliasTarget {
        amount: 1,
        all: false,
        types: Vec::new(),
    });

    let mut enum_type = match type_syntax("enum", &[], 10) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    enum_type.has_parser = true;
    enum_type.enum_values = vec!["zombie".to_owned()];
    parts.syntaxes.push(Syntax::Type(enum_type));
    let catalog = Catalog::new(parts);

    let plural = catalog.type_literal_matches("zombies").collect::<Vec<_>>();
    assert_eq!(plural.len(), 1);
    assert_eq!(plural[0].type_info.code_name.as_str(), "itemtype");
    assert_eq!(plural[0].source, TypeLiteralSource::Alias);
    assert!(plural[0].plural);

    let enum_plural = catalog.type_literal_matches("zombies").any(|matched| {
        matched.type_info.code_name.as_str() == "enum"
            && matched.source == TypeLiteralSource::EnumConstant
    });
    assert!(!enum_plural);
}

#[test]
fn detailed_matches_keep_type_parse_order_and_source_order() {
    let mut parts = parts();
    let mut later = match type_syntax("later", &[], 20) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    later.has_parser = true;
    later.parser_patterns = vec!["shared".to_owned()];
    later.literal_values = vec!["shared".to_owned()];

    let mut earlier = match type_syntax("earlier", &[], 10) {
        Syntax::Type(value) => value,
        _ => unreachable!(),
    };
    earlier.has_parser = true;
    earlier.enum_values = vec!["shared".to_owned()];
    parts.syntaxes = vec![Syntax::Type(later), Syntax::Type(earlier)];
    let catalog = Catalog::new(parts);

    let matches = catalog
        .type_literal_matches("shared")
        .map(|matched| {
            (
                matched.type_info.code_name.as_str(),
                matched.source,
                matched.plural,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matches,
        vec![
            ("earlier", TypeLiteralSource::EnumConstant, false),
            ("later", TypeLiteralSource::ParserPattern, false),
            ("later", TypeLiteralSource::Supplier, false),
        ]
    );
}

#[test]
fn declared_method_queries_preserve_unavailable_and_exact_signatures() {
    let mut parts = parts();
    let mut player = class("org.bukkit.entity.Player", None, &[]);
    player.methods = Some(vec![ClassMethod {
        name: "transfer".to_owned(),
        parameter_types: vec![class_name("java.lang.String"), class_name("int")],
        return_type: class_name("void"),
        is_static: false,
    }]);
    parts.classes.push(player);
    let catalog = Catalog::new(parts);

    assert_eq!(
        catalog.declared_method_exists(
            "org.bukkit.entity.Player",
            "transfer",
            &["java.lang.String", "int"],
            None,
        ),
        Some(true)
    );
    assert_eq!(
        catalog.declared_method_exists(
            "org.bukkit.entity.Player",
            "transfer",
            &["java.lang.String"],
            None,
        ),
        Some(false)
    );
    assert_eq!(
        catalog.declared_method_exists("java.lang.Object", "toString", &[], None),
        None
    );
}
