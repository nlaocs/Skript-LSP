use super::*;
use std::path::Path;

use tempfile::TempDir;

const PROJECT: &str = "file:///workspace";
const DOCUMENT: &str = "file:///workspace/test.sk";
const OTHER_DOCUMENT: &str = "file:///workspace/other.sk";
const OWNER: &str = "example.owner";
const READER: &str = "example.reader";
const WRITER: &str = "example.writer";
const SHARED: &str = "symbols";
const OTHER: &str = "other-symbols";
const PRIVATE: &str = "cache";
const SCHEMA: &str = "example.symbols";

fn make_store(directory: &Path, max_namespace_bytes: usize) -> StateStore {
    let mut quotas = StateQuotas::default();
    quotas.max_namespace_bytes = max_namespace_bytes;
    StateStore::new(StateStoreConfig {
        data_directory: Some(directory.to_owned()),
        quotas,
    })
    .expect("StateStore must initialize")
}

fn register_namespaces(store: &StateStore) {
    store
        .register_component(
            OWNER,
            &[
                NamespaceDeclaration::shared(SHARED, SCHEMA, 1, [READER], [WRITER]),
                NamespaceDeclaration::shared(OTHER, SCHEMA, 1, [READER], [WRITER]),
                NamespaceDeclaration::private(PRIVATE, "example.cache", 1),
            ],
        )
        .expect("test namespaces must register");
}

fn begin(store: &StateStore, document: &str, revision: u64) -> ParseTransaction {
    store
        .begin_parse(PROJECT, document, revision)
        .expect("parse transaction must begin")
}

fn value(text: &str) -> StateValue {
    StateValue::new(SCHEMA, StateEncoding::Json, text.as_bytes().to_vec())
}

fn put_in(parse: &ParseTransaction, scope: StateScope, namespace: &str, key: &str, text: &str) {
    let mut invocation = parse
        .begin_invocation(OWNER)
        .expect("owner invocation must begin");
    invocation
        .put(
            scope,
            NamespaceVisibility::Shared,
            namespace,
            key,
            value(text),
        )
        .expect("test write must stage");
    invocation.commit().expect("test write must merge");
}

fn put(parse: &ParseTransaction, scope: StateScope, key: &str, text: &str) {
    put_in(parse, scope, SHARED, key, text);
}

fn stage_delete(parse: &ParseTransaction, scope: StateScope, key: &str) -> bool {
    let mut invocation = parse
        .begin_invocation(OWNER)
        .expect("owner invocation must begin");
    let existed = invocation
        .delete(scope, NamespaceVisibility::Shared, SHARED, key)
        .expect("test delete must stage");
    invocation.commit().expect("test delete must merge");
    existed
}

fn visible_in(
    parse: &ParseTransaction,
    scope: StateScope,
    namespace: &str,
    key: &str,
) -> Option<StateValue> {
    let mut invocation = parse
        .begin_invocation(OWNER)
        .expect("owner reader invocation must begin");
    let result = invocation
        .get(scope, NamespaceVisibility::Shared, namespace, key)
        .expect("test read must succeed");
    invocation.rollback();
    result
}

fn visible(parse: &ParseTransaction, scope: StateScope, key: &str) -> Option<StateValue> {
    visible_in(parse, scope, SHARED, key)
}

fn read_and_commit(
    parse: &ParseTransaction,
    scope: StateScope,
    namespace: &str,
    key: &str,
) -> Option<StateValue> {
    let mut invocation = parse
        .begin_invocation(OWNER)
        .expect("owner reader invocation must begin");
    let result = invocation
        .get(scope, NamespaceVisibility::Shared, namespace, key)
        .expect("test read must succeed");
    invocation.commit().expect("test read must merge");
    result
}

#[test]
fn deferred_writes_stay_absent_until_one_candidate_is_selected() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let base = parse.savepoint().expect("candidate base must capture");

    put(&parse, StateScope::Parse, "selected", "yes");
    let selected = parse
        .defer_since(&base)
        .expect("selected candidate must defer");
    assert_eq!(visible(&parse, StateScope::Parse, "selected"), None);

    put(&parse, StateScope::Parse, "rejected", "no");
    let rejected = parse
        .defer_since(&base)
        .expect("rejected candidate must defer");
    assert_ne!(selected.id, rejected.id);
    assert_eq!(selected.writes.len(), 1);
    assert_eq!(rejected.writes.len(), 1);
    assert_eq!(visible(&parse, StateScope::Parse, "selected"), None);
    assert_eq!(visible(&parse, StateScope::Parse, "rejected"), None);

    assert!(
        parse
            .apply_delta(&selected)
            .expect("selected candidate must apply")
    );
    assert_eq!(
        visible(&parse, StateScope::Parse, "selected"),
        Some(value("yes"))
    );
    assert_eq!(visible(&parse, StateScope::Parse, "rejected"), None);
    parse.cancel().expect("parse must cancel");
}

#[test]
fn applying_a_delta_is_idempotent_and_rollback_restores_its_receipt() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let base = parse.savepoint().expect("candidate base must capture");

    put(&parse, StateScope::Parse, "once", "value");
    let delta = parse.defer_since(&base).expect("candidate must defer");

    assert!(parse.apply_delta(&delta).expect("first apply must succeed"));
    assert!(
        !parse
            .apply_delta(&delta)
            .expect("second apply must be a no-op")
    );
    assert_eq!(
        visible(&parse, StateScope::Parse, "once"),
        Some(value("value"))
    );

    parse
        .rollback_to(&base)
        .expect("rollback must restore the pre-apply receipt");
    assert_eq!(visible(&parse, StateScope::Parse, "once"), None);
    assert!(
        parse
            .apply_delta(&delta)
            .expect("delta must be re-applicable after rollback")
    );
    assert_eq!(
        visible(&parse, StateScope::Parse, "once"),
        Some(value("value"))
    );
    parse.cancel().expect("parse must cancel");
}

#[test]
fn nested_candidate_aggregates_subsume_child_receipts() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let parent_base = parse.savepoint().expect("parent base must capture");
    let child_base = parse.savepoint().expect("child base must capture");

    put(&parse, StateScope::Parse, "child", "child-value");
    let child = parse.defer_since(&child_base).expect("child must defer");
    assert!(parse.apply_delta(&child).expect("child must apply"));

    put(&parse, StateScope::Parse, "parent", "parent-value");
    let parent = parse
        .defer_since(&parent_base)
        .expect("parent aggregate must defer");
    assert!(parent.nested.contains(&child.id));
    assert_eq!(parent.writes.len(), 2);

    assert!(parse.apply_delta(&parent).expect("parent must apply"));
    assert!(
        !parse
            .apply_delta(&child)
            .expect("subsumed child must be idempotent")
    );
    assert_eq!(
        visible(&parse, StateScope::Parse, "child"),
        Some(value("child-value"))
    );
    assert_eq!(
        visible(&parse, StateScope::Parse, "parent"),
        Some(value("parent-value"))
    );
    parse.cancel().expect("parse must cancel");
}

#[test]
fn stale_namespace_reads_reject_delta_application() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let base = parse.savepoint().expect("candidate base must capture");

    assert_eq!(
        read_and_commit(&parse, StateScope::Project, SHARED, "observed"),
        None
    );
    let stale = parse.defer_since(&base).expect("read candidate must defer");
    assert!(stale.writes.is_empty());

    put(&parse, StateScope::Project, "changed", "new");
    assert!(matches!(
        parse.apply_delta(&stale),
        Err(StateError::TransactionConflict { namespace }) if namespace == SHARED
    ));
    assert_eq!(
        visible(&parse, StateScope::Project, "changed"),
        Some(value("new"))
    );
    parse.cancel().expect("parse must cancel");
}

#[test]
fn applying_a_write_delta_enforces_the_combined_namespace_quota() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), 30);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let base = parse.savepoint().expect("candidate base must capture");

    put(&parse, StateScope::Parse, "a", "x");
    let mut candidate = parse.defer_since(&base).expect("candidate must defer");
    // Isolate merge-time quota validation from the stale-read regression. The
    // candidate is intentionally write-only for this test.
    candidate.observed_revisions.clear();
    put(&parse, StateScope::Parse, "b", "x");

    assert!(matches!(
        parse.apply_delta(&candidate),
        Err(StateError::QuotaExceeded { .. })
    ));
    assert_eq!(visible(&parse, StateScope::Parse, "b"), Some(value("x")));
    assert_eq!(visible(&parse, StateScope::Parse, "a"), None);
    parse.cancel().expect("parse must cancel");
}

#[test]
fn foreign_savepoints_and_deltas_are_rejected_across_transactions_and_stores() {
    let first_directory = TempDir::new().expect("temporary directory must exist");
    let second_directory = TempDir::new().expect("temporary directory must exist");
    let first_store = make_store(
        first_directory.path(),
        StateQuotas::default().max_namespace_bytes,
    );
    let second_store = make_store(
        second_directory.path(),
        StateQuotas::default().max_namespace_bytes,
    );
    register_namespaces(&first_store);
    register_namespaces(&second_store);

    let first = begin(&first_store, DOCUMENT, 1);
    let same_store = begin(&first_store, OTHER_DOCUMENT, 1);
    let other_store = begin(&second_store, DOCUMENT, 1);
    let savepoint = first
        .savepoint()
        .expect("foreign-test savepoint must capture");

    assert!(matches!(
        same_store.rollback_to(&savepoint),
        Err(StateError::ForeignSavepoint)
    ));
    assert!(matches!(
        other_store.rollback_to(&savepoint),
        Err(StateError::ForeignSavepoint)
    ));
    assert!(matches!(
        same_store.defer_since(&savepoint),
        Err(StateError::ForeignSavepoint)
    ));
    assert!(matches!(
        other_store.defer_since(&savepoint),
        Err(StateError::ForeignSavepoint)
    ));

    put(&first, StateScope::Parse, "foreign", "value");
    let delta = first
        .defer_since(&savepoint)
        .expect("source delta must defer");
    assert!(matches!(
        same_store.apply_delta(&delta),
        Err(StateError::ForeignSavepoint)
    ));
    assert!(matches!(
        other_store.apply_delta(&delta),
        Err(StateError::ForeignSavepoint)
    ));

    first.cancel().expect("source parse must cancel");
    same_store.cancel().expect("same-store parse must cancel");
    other_store.cancel().expect("cross-store parse must cancel");
}

#[test]
fn deferred_deletes_apply_and_publish_to_the_next_revision() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);

    let seed = begin(&store, DOCUMENT, 1);
    put(&seed, StateScope::Project, "gone", "before");
    seed.commit().expect("seed value must commit");

    let parse = begin(&store, DOCUMENT, 2);
    let base = parse.savepoint().expect("delete base must capture");
    assert!(stage_delete(&parse, StateScope::Project, "gone"));
    let deletion = parse
        .defer_since(&base)
        .expect("delete candidate must defer");
    assert_eq!(deletion.writes.len(), 1);
    assert!(parse.apply_delta(&deletion).expect("delete must apply"));
    assert_eq!(visible(&parse, StateScope::Project, "gone"), None);
    parse.commit().expect("delete must commit");

    let next = begin(&store, DOCUMENT, 3);
    assert_eq!(visible(&next, StateScope::Project, "gone"), None);
    next.cancel().expect("next parse must cancel");
}

#[test]
fn overlay_revisions_are_monotonic_when_divergent_deltas_are_selected() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let base = parse.savepoint().expect("branch base must capture");
    let original_revision = parse.state_revision().expect("revision must be readable");
    assert_eq!(original_revision, 0);

    put_in(&parse, StateScope::Parse, SHARED, "left", "left-value");
    let left = parse.defer_since(&base).expect("left branch must defer");
    assert_eq!(parse.state_revision().unwrap(), original_revision);

    put_in(&parse, StateScope::Parse, OTHER, "right", "right-value");
    let right = parse.defer_since(&base).expect("right branch must defer");
    assert_eq!(parse.state_revision().unwrap(), original_revision);

    assert!(parse.apply_delta(&left).expect("left branch must apply"));
    let left_revision = parse
        .state_revision()
        .expect("left revision must be readable");
    assert!(left_revision > original_revision);
    assert!(parse.apply_delta(&right).expect("right branch must apply"));
    let right_revision = parse
        .state_revision()
        .expect("right revision must be readable");
    assert!(right_revision > left_revision);
    assert!(
        !parse
            .apply_delta(&left)
            .expect("left reapply must be a no-op")
    );
    assert_eq!(parse.state_revision().unwrap(), right_revision);
    assert_eq!(
        visible_in(&parse, StateScope::Parse, SHARED, "left"),
        Some(value("left-value"))
    );
    assert_eq!(
        visible_in(&parse, StateScope::Parse, OTHER, "right"),
        Some(value("right-value"))
    );
    parse.cancel().expect("parse must cancel");
}

#[test]
fn divergent_same_key_branches_receive_distinct_revisions_after_rollback() {
    let directory = TempDir::new().expect("temporary directory must exist");
    let store = make_store(directory.path(), StateQuotas::default().max_namespace_bytes);
    register_namespaces(&store);
    let parse = begin(&store, DOCUMENT, 1);
    let base = parse.savepoint().expect("same-key base must capture");
    let original_revision = parse.state_revision().expect("revision must be readable");
    assert_eq!(original_revision, 0);

    put(&parse, StateScope::Parse, "same-key", "left");
    let left = parse
        .defer_since(&base)
        .expect("left same-key branch must defer");
    assert_eq!(parse.state_revision().unwrap(), original_revision);
    assert!(
        parse
            .apply_delta(&left)
            .expect("left same-key branch must apply")
    );
    let left_revision = parse
        .state_revision()
        .expect("left revision must be readable");
    assert!(left_revision > original_revision);

    parse
        .rollback_to(&base)
        .expect("rollback must restore the original branch revision");
    assert_eq!(parse.state_revision().unwrap(), original_revision);

    put(&parse, StateScope::Parse, "same-key", "right");
    let right = parse
        .defer_since(&base)
        .expect("right same-key branch must defer");
    assert_eq!(parse.state_revision().unwrap(), original_revision);
    assert!(
        parse
            .apply_delta(&right)
            .expect("right same-key branch must apply")
    );
    let right_revision = parse
        .state_revision()
        .expect("right revision must be readable");
    assert!(right_revision > left_revision);
    assert_eq!(
        visible(&parse, StateScope::Parse, "same-key"),
        Some(value("right"))
    );
    parse.cancel().expect("parse must cancel");
}
