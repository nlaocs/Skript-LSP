use skript_parser::{
    FunctionDeclaration, FunctionParameterDeclaration, FunctionRegistryError,
    FunctionRegistryTransaction, FunctionReturnContract, FunctionScope, FunctionVersionPolicy,
    TextRange,
};
use syntaxes::ClassName;

fn number() -> ClassName {
    ClassName("java.lang.Number".to_owned())
}

fn declaration(
    name: &str,
    scope: FunctionScope,
    parameters: Vec<FunctionParameterDeclaration>,
) -> FunctionDeclaration {
    FunctionDeclaration::new(
        format!("function {name}(...)"),
        TextRange::new(0, format!("function {name}(...)").len()),
        scope,
        name,
        parameters,
        FunctionReturnContract::single(number()),
    )
}

fn required(name: &str) -> FunctionParameterDeclaration {
    FunctionParameterDeclaration::required(name, number(), true)
}

#[test]
fn version_policy_tracks_each_upstream_function_feature_boundary() {
    let v26 = FunctionVersionPolicy::for_skript_version(2, 6, 4);
    assert!(!v26.allow_local_functions);
    assert!(!v26.allow_overloads);
    assert!(!v26.allow_returns_keyword);
    assert!(!v26.allow_leading_underscore);
    assert!(!v26.allow_named_arguments);
    assert!(!v26.allow_arrow_return);

    assert!(FunctionVersionPolicy::for_skript_version(2, 7, 0).allow_local_functions);
    assert!(FunctionVersionPolicy::for_skript_version(2, 8, 0).allow_returns_keyword);
    assert!(FunctionVersionPolicy::for_skript_version(2, 12, 0).allow_overloads);
    assert!(FunctionVersionPolicy::for_skript_version(2, 12, 0).allow_leading_underscore);
    let v214 = FunctionVersionPolicy::for_skript_version(2, 14, 0);
    assert!(v214.allow_named_arguments);
    assert!(v214.allow_arrow_return);
    assert!(!v214.wide_named_argument_names);
    assert!(FunctionVersionPolicy::for_skript_version(2, 15, 0).wide_named_argument_names);

    assert!(
        !FunctionVersionPolicy::for_skript_version_with_case_insensitive_variables(2, 8, 0, false)
            .case_insensitive_parameters
    );
    assert!(
        FunctionVersionPolicy::for_skript_version_with_case_insensitive_variables(2, 7, 0, false)
            .case_insensitive_parameters,
        "pre-2.8 duplicate checks ignore the later configuration"
    );
}

#[test]
fn parameter_duplicates_follow_the_effective_case_setting() {
    let duplicate = declaration(
        "value",
        FunctionScope::Global,
        vec![required("Input"), required("input")],
    );
    let mut default_policy = FunctionRegistryTransaction::with_default_policy("doc", 1);
    assert!(matches!(
        default_policy.register(duplicate.clone()),
        Err(FunctionRegistryError::DuplicateParameter { .. })
    ));

    let policy =
        FunctionVersionPolicy::for_skript_version_with_case_insensitive_variables(2, 15, 4, false);
    FunctionRegistryTransaction::new("doc", 1, policy)
        .register(duplicate)
        .expect("case-sensitive variables allow differently-cased parameter names");
}

#[test]
fn leading_underscore_is_rejected_until_skript_2_12() {
    let declaration = declaration("_value", FunctionScope::Global, Vec::new());
    let mut old = FunctionRegistryTransaction::new(
        "doc",
        1,
        FunctionVersionPolicy::for_skript_version(2, 11, 0),
    );
    assert!(matches!(
        old.register(declaration.clone()),
        Err(FunctionRegistryError::InvalidFunctionName)
    ));
    FunctionRegistryTransaction::new(
        "doc",
        1,
        FunctionVersionPolicy::for_skript_version(2, 12, 0),
    )
    .register(declaration)
    .expect("Skript 2.12 accepts a leading underscore");
}

#[test]
fn return_spelling_follows_skript_version_boundaries() {
    let returns = declaration("value", FunctionScope::Global, Vec::new())
        .with_metadata("function.return-syntax", "returns");
    let mut v27 = FunctionRegistryTransaction::new(
        "doc",
        1,
        FunctionVersionPolicy::for_skript_version(2, 7, 0),
    );
    assert!(matches!(
        v27.register(returns.clone()),
        Err(FunctionRegistryError::ReturnsKeywordUnsupported { .. })
    ));
    FunctionRegistryTransaction::new("doc", 1, FunctionVersionPolicy::for_skript_version(2, 8, 0))
        .register(returns)
        .expect("Skript 2.8 accepts `returns`");

    let arrow = declaration("arrow", FunctionScope::Global, Vec::new())
        .with_metadata("function.return-syntax", "arrow");
    let mut v213 = FunctionRegistryTransaction::new(
        "doc",
        1,
        FunctionVersionPolicy::for_skript_version(2, 13, 0),
    );
    assert!(matches!(
        v213.register(arrow.clone()),
        Err(FunctionRegistryError::ArrowReturnUnsupported { .. })
    ));
    FunctionRegistryTransaction::new(
        "doc",
        1,
        FunctionVersionPolicy::for_skript_version(2, 14, 0),
    )
    .register(arrow)
    .expect("Skript 2.14 accepts `->`");
}

#[test]
fn modern_declaration_keeps_owned_source_and_projects_call_signature() {
    let mut transaction = FunctionRegistryTransaction::new(
        "file:///workspace/test.sk",
        7,
        FunctionVersionPolicy::SKRIPT_2_15_4,
    );
    let registration = transaction
        .register(
            declaration(
                "scale",
                FunctionScope::Local,
                vec![FunctionParameterDeclaration::defaulted(
                    "factor",
                    number(),
                    true,
                    "2",
                )],
            )
            .with_metadata("addon", "test"),
        )
        .expect("modern declaration must register");

    assert_eq!(registration.declaration.source, "function scale(...)");
    assert_eq!(
        registration.declaration.parameters[0]
            .default_source
            .as_deref(),
        Some("2")
    );
    assert_eq!(registration.definition.parser_id, "document.function");
    assert!(registration.definition.parameters[0].optional);
    assert_eq!(
        registration
            .definition
            .metadata
            .get("function.scope")
            .map(String::as_str),
        Some("local")
    );
    assert_eq!(
        registration
            .definition
            .metadata
            .get("function.parameter.0.default-source")
            .map(String::as_str),
        Some("2")
    );
}

#[test]
fn policy_capabilities_are_carried_into_the_call_projection() {
    let policy = FunctionVersionPolicy {
        allow_named_arguments: false,
        ..FunctionVersionPolicy::SKRIPT_2_15_4
    };
    let mut transaction = FunctionRegistryTransaction::new("doc", 1, policy);
    let registration = transaction
        .register(declaration(
            "collect",
            FunctionScope::Global,
            vec![FunctionParameterDeclaration::required(
                "values",
                number(),
                false,
            )],
        ))
        .unwrap();

    assert!(
        !registration
            .definition
            .parameters
            .first()
            .expect("the plural parameter must be projected")
            .single
    );
    assert_eq!(
        registration
            .definition
            .metadata
            .get("function.named-arguments")
            .map(String::as_str),
        Some("false")
    );
    assert_eq!(
        registration
            .definition
            .metadata
            .get("function.named-argument-pattern")
            .map(String::as_str),
        Some("wide")
    );
}

#[test]
fn local_lookup_precedes_global_and_only_shadows_same_shape() {
    let mut transaction = FunctionRegistryTransaction::with_default_policy("doc", 3);
    transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .unwrap();
    transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![FunctionParameterDeclaration::required("x", number(), false)],
        ))
        .unwrap();
    transaction
        .register(declaration(
            "value",
            FunctionScope::Local,
            vec![required("x")],
        ))
        .unwrap();
    let snapshot = transaction.freeze().unwrap();

    let local_lookup = snapshot.lookup("value", FunctionScope::Local);
    assert_eq!(local_lookup.len(), 2);
    assert_eq!(
        local_lookup[0]
            .metadata
            .get("function.scope")
            .map(String::as_str),
        Some("local")
    );
    assert_eq!(
        local_lookup[1]
            .metadata
            .get("function.scope")
            .map(String::as_str),
        Some("global")
    );
    assert_eq!(snapshot.lookup("value", FunctionScope::Global).len(), 2);
}

#[test]
fn modern_policy_rejects_an_identical_signature_but_accepts_an_overload() {
    let mut transaction = FunctionRegistryTransaction::with_default_policy("doc", 1);
    transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .unwrap();
    let duplicate = transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .expect_err("the same scope cannot register an identical signature twice");
    assert!(matches!(
        duplicate,
        FunctionRegistryError::DuplicateSignature {
            scope: FunctionScope::Global,
            ..
        }
    ));

    transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![FunctionParameterDeclaration::required("x", number(), false)],
        ))
        .expect("a different plurality is a valid overload in modern Skript");
}

#[test]
fn legacy_policy_is_global_only_and_rejects_overloads() {
    let mut transaction =
        FunctionRegistryTransaction::new("legacy.sk", 1, FunctionVersionPolicy::SKRIPT_2_6_4);
    let local_error = transaction
        .register(declaration(
            "value",
            FunctionScope::Local,
            vec![required("x")],
        ))
        .expect_err("2.6.4 has no local Function declarations");
    assert!(matches!(
        local_error,
        FunctionRegistryError::LocalFunctionsUnsupported { .. }
    ));

    transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .unwrap();
    let overload_error = transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![FunctionParameterDeclaration::required("x", number(), false)],
        ))
        .expect_err("2.6.4 has no overloaded global signatures");
    assert!(matches!(
        overload_error,
        FunctionRegistryError::OverloadsUnsupported { .. }
    ));
}

#[test]
fn savepoint_rollback_removes_later_registrations_only() {
    let mut transaction = FunctionRegistryTransaction::with_default_policy("doc", 4);
    transaction
        .register(declaration(
            "first",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .unwrap();
    let savepoint = transaction.savepoint();
    transaction
        .register(declaration(
            "second",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .unwrap();
    transaction
        .rollback(savepoint)
        .expect("savepoint from the same transaction must roll back");
    let snapshot = transaction.freeze().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot.lookup("first", FunctionScope::Global).len(), 1);
    assert!(snapshot.lookup("second", FunctionScope::Global).is_empty());
}

#[test]
fn rollback_rejects_a_savepoint_from_another_transaction() {
    let mut first = FunctionRegistryTransaction::with_default_policy("doc", 1);
    let second = FunctionRegistryTransaction::with_default_policy("doc", 1);
    let foreign = second.savepoint();

    assert!(matches!(
        first.rollback(foreign),
        Err(FunctionRegistryError::InvalidSavepoint)
    ));
}

#[test]
fn freeze_rejects_mutation_and_preserves_document_revision() {
    let mut transaction = FunctionRegistryTransaction::with_default_policy("doc", 99);
    transaction
        .register(declaration(
            "value",
            FunctionScope::Global,
            vec![required("x")],
        ))
        .unwrap();
    let savepoint = transaction.savepoint();
    let snapshot = transaction
        .freeze()
        .expect("open transaction must freeze once");

    assert_eq!(snapshot.document_id(), "doc");
    assert_eq!(snapshot.revision(), 99);
    assert!(transaction.is_frozen());
    assert!(matches!(
        transaction.rollback(savepoint),
        Err(FunctionRegistryError::Frozen)
    ));
    assert!(matches!(
        transaction.register(declaration(
            "later",
            FunctionScope::Global,
            vec![required("x")]
        )),
        Err(FunctionRegistryError::Frozen)
    ));
    assert!(matches!(
        transaction.freeze(),
        Err(FunctionRegistryError::Frozen)
    ));
}

#[test]
fn invalid_declarations_do_not_mutate_the_transaction() {
    let mut transaction = FunctionRegistryTransaction::with_default_policy("doc", 1);
    let invalid = FunctionDeclaration::new(
        "function broken",
        TextRange::new(0, 100),
        FunctionScope::Global,
        "broken",
        vec![required("x"), required("x")],
        FunctionReturnContract::none(),
    );
    assert!(transaction.register(invalid).is_err());
    assert_eq!(transaction.savepoint().registration_count(), 0);
}
