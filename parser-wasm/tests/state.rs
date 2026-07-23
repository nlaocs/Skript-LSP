use std::path::Path;

use parser_wasm::state::{
    NamespaceDeclaration, NamespaceVisibility, StateEncoding, StateError, StateQuotas, StateScope,
    StateStore, StateStoreConfig, StateValue,
};
use tempfile::TempDir;

const PROJECT: &str = "file:///workspace";
const DOCUMENT: &str = "file:///workspace/test.sk";
const OWNER: &str = "example.owner";
const READER: &str = "example.reader";
const WRITER: &str = "example.writer";
const STRANGER: &str = "example.stranger";
const SHARED: &str = "symbols";
const PRIVATE: &str = "cache";
const SCHEMA: &str = "example.symbols";

fn config(path: &Path) -> StateStoreConfig {
    StateStoreConfig {
        data_directory: Some(path.to_owned()),
        quotas: StateQuotas::default(),
    }
}

fn store(path: &Path) -> StateStore {
    StateStore::new(config(path)).expect("StateStore must initialize")
}

fn register_namespaces(store: &StateStore, schema_version: u32) {
    store
        .register_component(
            OWNER,
            &[
                NamespaceDeclaration::shared(SHARED, SCHEMA, schema_version, [READER], [WRITER]),
                NamespaceDeclaration::private(PRIVATE, "example.cache", 1),
            ],
        )
        .expect("owner namespaces must register");
}

fn value(text: &str) -> StateValue {
    StateValue::new(SCHEMA, StateEncoding::Json, text.as_bytes())
}

fn read(
    store: &StateStore,
    component: &str,
    scope: StateScope,
    visibility: NamespaceVisibility,
    namespace: &str,
    key: &str,
    revision: u64,
) -> Result<Option<StateValue>, StateError> {
    let parse = store.begin_parse(PROJECT, DOCUMENT, revision)?;
    let mut invocation = parse.begin_invocation(component)?;
    let result = invocation.get(scope, visibility, namespace, key)?;
    invocation.commit()?;
    parse.commit()?;
    Ok(result)
}

#[test]
fn shares_parse_overlay_between_authorized_addons() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = store(directory.path());
    register_namespaces(&store, 1);
    let parse = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("parse must begin");

    let mut owner = parse.begin_invocation(OWNER).expect("owner may invoke");
    owner
        .put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "variable::name",
            value("number"),
        )
        .expect("owner may write");
    owner.commit().expect("owner state must merge");

    let mut reader = parse.begin_invocation(READER).expect("reader may invoke");
    let stored = reader
        .get(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "variable::name",
        )
        .expect("reader may read")
        .expect("merged parse value must be visible");
    assert_eq!(stored, value("number"));
    reader.commit().expect("reader state must merge");
    let accesses = parse
        .read_write_set()
        .expect("accepted invocations must expose their access set");
    assert!(
        accesses
            .reads
            .iter()
            .any(|entry| entry.key == "variable::name")
    );
    assert!(
        accesses
            .writes
            .iter()
            .any(|entry| entry.key == "variable::name")
    );
    assert_eq!(accesses.namespace_revisions.len(), 1);
    parse.commit().expect("parse must commit");
}

#[test]
fn enforces_private_and_shared_namespace_permissions() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = store(directory.path());
    register_namespaces(&store, 1);
    let parse = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("parse must begin");

    let mut reader = parse.begin_invocation(READER).expect("reader may invoke");
    let error = reader
        .put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "key",
            value("value"),
        )
        .expect_err("reader must not write");
    assert!(matches!(error, StateError::AccessDenied { .. }));
    reader.rollback();

    let mut stranger = parse
        .begin_invocation(STRANGER)
        .expect("stranger may invoke");
    let error = stranger
        .get(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "key",
        )
        .expect_err("undeclared reader must not read shared state");
    assert!(matches!(error, StateError::AccessDenied { .. }));
    let error = stranger
        .get(
            StateScope::Parse,
            NamespaceVisibility::Private,
            PRIVATE,
            "key",
        )
        .expect_err("another component must not address private state");
    assert!(matches!(error, StateError::UnknownNamespace { .. }));
    stranger.rollback();

    let mut writer = parse.begin_invocation(WRITER).expect("writer may invoke");
    writer
        .put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "key",
            value("value"),
        )
        .expect("declared writer may write");
    writer.commit().expect("writer state must merge");
    parse.commit().expect("parse must commit");
}

#[test]
fn rolls_back_invocations_and_candidate_savepoints() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = store(directory.path());
    register_namespaces(&store, 1);
    let parse = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("parse must begin");

    let mut rejected = parse.begin_invocation(OWNER).expect("owner may invoke");
    rejected
        .put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "rejected",
            value("discarded"),
        )
        .expect("temporary write must succeed");
    rejected.rollback();

    let savepoint = parse.savepoint().expect("savepoint must be captured");
    let mut accepted = parse.begin_invocation(OWNER).expect("owner may invoke");
    accepted
        .put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "candidate",
            value("discarded"),
        )
        .expect("candidate write must succeed");
    accepted.commit().expect("candidate state must merge first");
    parse
        .rollback_to(&savepoint)
        .expect("rejected candidate must roll back to its savepoint");

    let mut observer = parse.begin_invocation(READER).expect("reader may invoke");
    for key in ["rejected", "candidate"] {
        assert_eq!(
            observer
                .get(StateScope::Parse, NamespaceVisibility::Shared, SHARED, key,)
                .expect("reader may inspect state"),
            None
        );
    }
    observer.commit().expect("observer state must merge");
    parse.commit().expect("parse must commit");
}

#[test]
fn compare_and_swap_is_atomic_within_the_parse_overlay() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = store(directory.path());
    register_namespaces(&store, 1);
    let parse = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("parse must begin");
    let mut invocation = parse.begin_invocation(OWNER).expect("owner may invoke");

    assert!(
        invocation
            .compare_and_swap(
                StateScope::Parse,
                NamespaceVisibility::Shared,
                SHARED,
                "key",
                None,
                Some(value("first")),
            )
            .expect("initial CAS must execute")
    );
    assert!(
        !invocation
            .compare_and_swap(
                StateScope::Parse,
                NamespaceVisibility::Shared,
                SHARED,
                "key",
                Some(&value("wrong")),
                Some(value("second")),
            )
            .expect("mismatched CAS must execute")
    );
    assert_eq!(
        invocation
            .get(
                StateScope::Parse,
                NamespaceVisibility::Shared,
                SHARED,
                "key",
            )
            .expect("value must remain readable"),
        Some(value("first"))
    );
    invocation.commit().expect("state must merge");
    parse.commit().expect("parse must commit");
}

#[test]
fn rejects_an_invocation_that_observed_an_older_parse_overlay() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = store(directory.path());
    register_namespaces(&store, 1);
    let parse = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("parse must begin");
    let mut first = parse.begin_invocation(OWNER).expect("owner may invoke");
    let mut second = parse.begin_invocation(OWNER).expect("owner may invoke");

    first
        .compare_and_swap(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "key",
            None,
            Some(value("first")),
        )
        .expect("first CAS must execute");
    second
        .compare_and_swap(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "key",
            None,
            Some(value("second")),
        )
        .expect("second CAS may stage against the same revision");

    first.commit().expect("first invocation must merge");
    assert!(matches!(
        second.commit(),
        Err(StateError::TransactionConflict { .. })
    ));
    let mut observer = parse.begin_invocation(READER).expect("reader may invoke");
    assert_eq!(
        observer
            .get(
                StateScope::Parse,
                NamespaceVisibility::Shared,
                SHARED,
                "key",
            )
            .expect("merged value must be readable"),
        Some(value("first"))
    );
    observer.commit().expect("observer state must merge");
    parse.commit().expect("parse must commit");
}

#[test]
fn rejects_stale_document_revisions_and_project_conflicts() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = store(directory.path());
    register_namespaces(&store, 1);

    let stale = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("old parse must begin");
    let mut stale_write = stale.begin_invocation(OWNER).expect("owner may invoke");
    stale_write
        .put(
            StateScope::Document,
            NamespaceVisibility::Shared,
            SHARED,
            "document",
            value("old"),
        )
        .expect("old parse may stage state");
    stale_write.commit().expect("old state may merge");
    let latest = store
        .begin_parse(PROJECT, DOCUMENT, 2)
        .expect("new parse must begin");
    assert!(matches!(
        stale.commit(),
        Err(StateError::StaleDocumentRevision { .. })
    ));
    latest.cancel().expect("latest parse may be cancelled");

    let first = store
        .begin_parse(PROJECT, "file:///workspace/a.sk", 1)
        .expect("first project parse must begin");
    let second = store
        .begin_parse(PROJECT, "file:///workspace/b.sk", 1)
        .expect("second project parse must begin");
    for (parse, text) in [(&first, "first"), (&second, "second")] {
        let mut invocation = parse.begin_invocation(OWNER).expect("owner may invoke");
        invocation
            .put(
                StateScope::Project,
                NamespaceVisibility::Shared,
                SHARED,
                "project",
                value(text),
            )
            .expect("project write must stage");
        invocation.commit().expect("project write must merge");
    }
    first.commit().expect("first project write must commit");
    assert!(matches!(
        second.commit(),
        Err(StateError::TransactionConflict { .. })
    ));
}

#[test]
fn persists_project_state_and_resets_changed_schemas() {
    let directory = TempDir::new().expect("temporary directory must exist");
    {
        let store = store(directory.path());
        register_namespaces(&store, 1);
        let parse = store
            .begin_parse(PROJECT, DOCUMENT, 1)
            .expect("parse must begin");
        let mut invocation = parse.begin_invocation(OWNER).expect("owner may invoke");
        invocation
            .put(
                StateScope::PersistentProject,
                NamespaceVisibility::Shared,
                SHARED,
                "persisted",
                value("number"),
            )
            .expect("persistent value must stage");
        invocation.commit().expect("persistent value must merge");
        parse.commit().expect("persistent value must commit");
    }
    {
        let store = store(directory.path());
        register_namespaces(&store, 1);
        assert_eq!(
            read(
                &store,
                READER,
                StateScope::PersistentProject,
                NamespaceVisibility::Shared,
                SHARED,
                "persisted",
                2,
            )
            .expect("persistent value must reload"),
            Some(value("number"))
        );
    }
    {
        let store = store(directory.path());
        register_namespaces(&store, 2);
        assert_eq!(
            read(
                &store,
                READER,
                StateScope::PersistentProject,
                NamespaceVisibility::Shared,
                SHARED,
                "persisted",
                3,
            )
            .expect("changed schema must remain readable"),
            None,
            "schema version changes must reset only that namespace"
        );
    }
}

#[test]
fn enforces_key_value_namespace_and_scan_quotas() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = StateStore::new(StateStoreConfig {
        data_directory: Some(directory.path().to_owned()),
        quotas: StateQuotas {
            max_key_bytes: 8,
            max_value_bytes: 20,
            max_namespace_bytes: 28,
            max_scan_entries: 1,
        },
    })
    .expect("quota store must initialize");
    register_namespaces(&store, 1);
    let parse = store
        .begin_parse(PROJECT, DOCUMENT, 1)
        .expect("parse must begin");
    let mut invocation = parse.begin_invocation(OWNER).expect("owner may invoke");

    assert!(matches!(
        invocation.put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "too-long-key",
            value("x")
        ),
        Err(StateError::QuotaExceeded { .. })
    ));
    assert!(matches!(
        invocation.put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "large",
            value("0123456789")
        ),
        Err(StateError::QuotaExceeded { .. })
    ));
    invocation
        .put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "a",
            value("1"),
        )
        .expect("first value must fit");
    assert!(matches!(
        invocation.put(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "b",
            value("2")
        ),
        Err(StateError::QuotaExceeded { .. })
    ));
    assert!(matches!(
        invocation.scan_prefix(
            StateScope::Parse,
            NamespaceVisibility::Shared,
            SHARED,
            "",
            2
        ),
        Err(StateError::QuotaExceeded { .. })
    ));
    invocation.rollback();
    parse.cancel().expect("parse may be cancelled");
}
