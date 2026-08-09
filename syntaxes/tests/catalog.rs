use std::collections::BTreeMap;
use syntax_pattern_parser::syntax::PluralRules;
use syntaxes::{
    Addon, AliasItem, AliasRegistry, AliasTarget, Catalog, CatalogParts, Class, ClassKind,
    ClassName, Converter, DefinitionId, Documentation, EventValue, Function, FunctionParameter,
    Noun, RegistrationId, Syntax, Type, TypeCodeName,
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
        default_expression_class: None,
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
    }
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
