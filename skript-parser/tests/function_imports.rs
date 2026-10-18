use skript_parser::{
    FunctionDeclaration, FunctionParameterDeclaration, FunctionRegistryError,
    FunctionRegistryTransaction, FunctionReturnContract, FunctionScope, FunctionVersionPolicy,
    ImportedFunction, TextRange,
};
use syntaxes::ClassName;

fn declaration(name: &str, scope: FunctionScope, class: &str) -> FunctionDeclaration {
    let source = format!("function {name}(value: type)");
    FunctionDeclaration::new(
        source.clone(),
        TextRange::new(0, source.len()),
        scope,
        name,
        vec![FunctionParameterDeclaration::required(
            "value",
            ClassName(class.into()),
            true,
        )],
        FunctionReturnContract::single(ClassName(class.into())),
    )
}
fn imported(document: &str, revision: u64, scope: FunctionScope, class: &str) -> ImportedFunction {
    let mut registry = FunctionRegistryTransaction::with_default_policy(document, revision);
    let definition = registry
        .register(declaration("projectValue", scope, class))
        .unwrap()
        .definition;
    ImportedFunction {
        definition,
        document_id: document.into(),
        document_revision: revision,
    }
}
#[test]
fn imported_global_is_read_only_preserves_identity_and_owner_revision() {
    let function = imported("source", 7, FunctionScope::Global, "java.lang.Number");
    let before = function.clone();
    let mut registry = FunctionRegistryTransaction::with_default_policy("caller", 3);
    registry.import_functions(vec![function]).unwrap();
    assert_eq!(
        registry.lookup_functions("projectValue", FunctionScope::Local),
        vec![before.definition.clone()]
    );
    let snapshot = registry.freeze().unwrap();
    assert!(snapshot.registrations().is_empty());
    assert_eq!(snapshot.definitions(), vec![before.definition.clone()]);
    assert_eq!(snapshot.imports(), &[before]);
    assert_eq!(snapshot.lookups()[0].name, "projectValue");
}
#[test]
fn local_shadowing_and_global_duplicates_follow_signature_policy() {
    let global = imported("source", 7, FunctionScope::Global, "java.lang.Number");
    let mut registry = FunctionRegistryTransaction::with_default_policy("caller", 3);
    registry.import_functions(vec![global.clone()]).unwrap();
    assert!(matches!(
        registry.register(declaration(
            "projectValue",
            FunctionScope::Global,
            "java.lang.Number"
        )),
        Err(FunctionRegistryError::DuplicateSignature { .. })
    ));
    let local = registry
        .register(declaration(
            "projectValue",
            FunctionScope::Local,
            "java.lang.Number",
        ))
        .unwrap();
    let different = registry
        .register(declaration(
            "projectValue",
            FunctionScope::Global,
            "java.lang.String",
        ))
        .unwrap();
    assert_eq!(
        registry.lookup_functions("projectValue", FunctionScope::Local),
        vec![local.definition, different.definition.clone()]
    );
    assert_eq!(
        registry.lookup_functions("projectValue", FunctionScope::Global),
        vec![different.definition, global.definition]
    );
}
#[test]
fn invalid_imports_are_atomic_and_cannot_forge_owner_revision_or_local_visibility() {
    for failure in ["owner", "revision", "local", "self", "identity"] {
        let mut function = imported("source", 7, FunctionScope::Global, "java.lang.Number");
        match failure {
            "owner" => function.document_id = "different".into(),
            "revision" => function.document_revision = 8,
            "local" => function
                .definition
                .metadata
                .insert("function.scope".into(), "local".into())
                .map(|_| ())
                .unwrap(),
            "self" => {
                function.document_id = "caller".into();
                function
                    .definition
                    .metadata
                    .insert("function.document-id".into(), "caller".into());
            }
            "identity" => function.definition.registration_id.clear(),
            _ => unreachable!(),
        }
        let mut registry = FunctionRegistryTransaction::with_default_policy("caller", 3);
        assert!(
            registry.import_functions(vec![function]).is_err(),
            "{failure}"
        );
        assert!(registry.callable_definitions().is_empty());
    }
}
#[test]
fn imported_duplicates_and_legacy_overloads_are_rejected() {
    let a = imported("sourceA", 1, FunctionScope::Global, "java.lang.Number");
    let b = imported("sourceB", 2, FunctionScope::Global, "java.lang.Number");
    let mut registry = FunctionRegistryTransaction::with_default_policy("caller", 3);
    assert!(matches!(
        registry.import_functions(vec![a.clone(), b]),
        Err(FunctionRegistryError::DuplicateSignature { .. })
    ));
    let b = imported("sourceB", 2, FunctionScope::Global, "java.lang.String");
    let mut old =
        FunctionRegistryTransaction::new("caller", 3, FunctionVersionPolicy::legacy_2_6_4());
    assert!(matches!(
        old.import_functions(vec![a, b]),
        Err(FunctionRegistryError::OverloadsUnsupported { .. })
    ));
}
#[test]
fn negative_and_rejected_lookups_survive_rollback_without_retaining_definitions() {
    let mut registry = FunctionRegistryTransaction::with_default_policy("caller", 3);
    registry.import_functions(Vec::new()).unwrap();
    assert!(
        registry
            .lookup_functions("later", FunctionScope::Local)
            .is_empty()
    );
    let savepoint = registry.savepoint();
    let declaration = registry
        .register(declaration(
            "later",
            FunctionScope::Global,
            "java.lang.Number",
        ))
        .unwrap();
    assert_eq!(
        registry.lookup_functions("later", FunctionScope::Local),
        vec![declaration.definition.clone()]
    );
    registry.rollback(savepoint).unwrap();
    let snapshot = registry.freeze().unwrap();
    assert!(snapshot.definitions().is_empty());
    assert_eq!(snapshot.lookups().len(), 3);
    assert!(snapshot.lookups()[0].definitions.is_empty());
    assert_eq!(
        snapshot.lookups()[2].definitions,
        vec![declaration.definition]
    );
}
