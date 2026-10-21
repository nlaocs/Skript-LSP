//! Complete State dependencies of explicitly cacheable, read-only hook calls.
//! Observations survive candidate rollback; adopted reads remain invocation-local.
use super::*;

const MAX_OBSERVATIONS: usize = 16_384;
const MAX_OBSERVED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
struct NamespaceObservation {
    component: String,
    key: RevisionKey,
    values: BTreeMap<String, StateValue>,
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "test.incremental";
    const DOCUMENT: &str = "file:///workspace/incremental.sk";
    const NAMESPACE: &str = "values";

    fn store() -> StateStore {
        let store = StateStore::new(StateStoreConfig::default()).unwrap();
        store
            .register_component(
                OWNER,
                &[NamespaceDeclaration::private(NAMESPACE, "test.values", 1)],
            )
            .unwrap();
        store
    }
    fn parse(store: &StateStore, revision: u64) -> ParseTransaction {
        let parse = store
            .begin_parse("file:///workspace", DOCUMENT, revision)
            .unwrap();
        parse.enable_read_observations().unwrap();
        parse
    }
    fn observed(parse: &ParseTransaction) -> (StateObservationCheckpoint, InvocationTransaction) {
        let before = parse.observation_checkpoint().unwrap();
        let mut invocation = parse.begin_invocation(OWNER).unwrap();
        invocation.observe_reads();
        (before, invocation)
    }
    fn put(parse: &ParseTransaction, key: &str, value: &str) {
        let mut invocation = parse.begin_invocation(OWNER).unwrap();
        invocation
            .put(
                StateScope::Parse,
                NamespaceVisibility::Private,
                NAMESPACE,
                key,
                StateValue::new("test.values", StateEncoding::Raw, value.as_bytes()),
            )
            .unwrap();
        invocation.commit().unwrap();
    }
    #[test]
    fn absent_read_and_empty_scan_invalidate_on_current_overlay_insertions() {
        for scan in [false, true] {
            let store = store();
            let first = parse(&store, 1);
            let (before, mut call) = observed(&first);
            if scan {
                assert!(
                    call.scan_prefix(
                        StateScope::Parse,
                        NamespaceVisibility::Private,
                        NAMESPACE,
                        "missing",
                        1
                    )
                    .unwrap()
                    .is_empty()
                );
            } else {
                assert!(
                    call.get(
                        StateScope::Parse,
                        NamespaceVisibility::Private,
                        NAMESPACE,
                        "missing"
                    )
                    .unwrap()
                    .is_none()
                );
            }
            let certificate = call.read_certificate(&before).unwrap().unwrap();
            call.rollback();
            first.cancel().unwrap();
            let second = parse(&store, 2);
            put(
                &second,
                if scan { "missing.child" } else { "missing" },
                "now-present",
            );
            let (_, mut replay) = observed(&second);
            let prior = second.read_write_set().unwrap();
            assert!(!replay.replay_read_certificate(&certificate).unwrap());
            assert!(replay.read_write_set().reads.is_empty());
            replay.rollback();
            assert_eq!(second.read_write_set().unwrap(), prior);
        }
    }
    #[test]
    fn equal_overlay_counter_is_not_a_read_value_certificate() {
        let store = store();
        let first = parse(&store, 1);
        put(&first, "key", "first");
        let old_revision = first.state_revision().unwrap();
        let (before, mut call) = observed(&first);
        call.get(
            StateScope::Parse,
            NamespaceVisibility::Private,
            NAMESPACE,
            "key",
        )
        .unwrap();
        let certificate = call.read_certificate(&before).unwrap().unwrap();
        call.rollback();
        first.cancel().unwrap();
        let second = parse(&store, 2);
        put(&second, "key", "second");
        assert_eq!(second.state_revision().unwrap(), old_revision);
        let (_, mut replay) = observed(&second);
        assert!(!replay.replay_read_certificate(&certificate).unwrap());
    }
    #[test]
    fn replay_uses_current_receipts_and_adopts_only_on_current_commit() {
        let store = store();
        let first = parse(&store, 1);
        let (before, mut call) = observed(&first);
        call.get(
            StateScope::Parse,
            NamespaceVisibility::Private,
            NAMESPACE,
            "absent",
        )
        .unwrap();
        let certificate = call.read_certificate(&before).unwrap().unwrap();
        call.rollback();
        first.cancel().unwrap();
        let second = parse(&store, 2);
        let mut fresh = second.begin_invocation(OWNER).unwrap();
        fresh
            .get(
                StateScope::Parse,
                NamespaceVisibility::Private,
                NAMESPACE,
                "absent",
            )
            .unwrap();
        let expected = fresh.read_write_set();
        fresh.rollback();
        let (_, mut replay) = observed(&second);
        assert!(replay.replay_read_certificate(&certificate).unwrap());
        assert_eq!(replay.read_write_set(), expected);
        assert!(second.read_write_set().unwrap().reads.is_empty());
        replay.rollback();
        assert!(second.read_write_set().unwrap().reads.is_empty());
        let (_, mut replay) = observed(&second);
        assert!(replay.replay_read_certificate(&certificate).unwrap());
        replay.commit().unwrap();
        assert_eq!(second.read_write_set().unwrap(), expected);
    }
    #[test]
    fn failed_compare_and_swap_and_read_errors_are_never_certified() {
        let store = store();
        let parse = parse(&store, 1);
        let (before, mut call) = observed(&parse);
        let unexpected =
            StateValue::new("test.values", StateEncoding::Raw, b"not-present".to_vec());
        assert!(
            !call
                .compare_and_swap(
                    StateScope::Parse,
                    NamespaceVisibility::Private,
                    NAMESPACE,
                    "missing",
                    Some(&unexpected),
                    None
                )
                .unwrap()
        );
        assert!(call.read_certificate(&before).unwrap().is_none());
        call.rollback();
        let (before, mut call) = observed(&parse);
        assert!(
            call.get(
                StateScope::Parse,
                NamespaceVisibility::Private,
                "unknown-namespace",
                "key"
            )
            .is_err()
        );
        assert!(call.read_certificate(&before).unwrap().is_none());
    }
    #[test]
    fn rejected_candidate_observations_survive_rollback_without_adopting_reads() {
        let store = store();
        let parse = parse(&store, 1);
        let checkpoint = parse.savepoint().unwrap();
        let (before, mut call) = observed(&parse);
        call.get(
            StateScope::Parse,
            NamespaceVisibility::Private,
            NAMESPACE,
            "missing",
        )
        .unwrap();
        let certificate = call.read_certificate(&before).unwrap().unwrap();
        call.rollback();
        parse.rollback_to(&checkpoint).unwrap();
        assert!(parse.read_write_set().unwrap().reads.is_empty());
        assert_eq!(parse.lock().unwrap().observations.entries.len(), 1);
        put(&parse, "missing", "changed");
        let (_, mut replay) = observed(&parse);
        assert!(!replay.replay_read_certificate(&certificate).unwrap());
    }
    #[test]
    fn foreign_checkpoints_and_unobserved_calls_are_rejected() {
        let store = store();
        let first = parse(&store, 1);
        let before = first.observation_checkpoint().unwrap();
        let call = first.begin_invocation(OWNER).unwrap();
        assert!(call.read_certificate(&before).unwrap().is_none());
        call.rollback();
        first.cancel().unwrap();
        let second = parse(&store, 2);
        let (_, call) = observed(&second);
        assert!(matches!(
            call.read_certificate(&before),
            Err(StateError::ForeignSavepoint)
        ));
    }
}

#[derive(Debug, Default)]
pub(super) struct ReadObservations {
    pub(super) enabled: bool,
    entries: Vec<NamespaceObservation>,
    bytes: usize,
    exhausted: bool,
    mutation_attempts: u64,
    failures: u64,
}

impl ReadObservations {
    pub(super) fn read(
        &mut self,
        component: &str,
        key: RevisionKey,
        values: &BTreeMap<String, StateValue>,
    ) {
        if !self.enabled || self.exhausted {
            return;
        }
        let bytes = namespace_size(values)
            .saturating_add(component.len())
            .saturating_add(key.namespace.name().len());
        if self.entries.len() >= MAX_OBSERVATIONS
            || self.bytes.saturating_add(bytes) > MAX_OBSERVED_BYTES
        {
            self.exhausted = true;
            return;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.push(NamespaceObservation {
            component: component.to_owned(),
            key,
            values: values.clone(),
        });
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StateObservationCheckpoint {
    transaction_id: Arc<()>,
    first: usize,
    mutations: u64,
    failures: u64,
}

/// Owned data only: no old overlay, revision counter, savepoint, or transaction
/// receipt is applied in another parse. Whole-namespace equality deliberately
/// invalidates on unrelated writes as well as absent reads and scan phantoms.
#[derive(Debug, Clone)]
pub(crate) struct StateReadCertificate {
    component: String,
    observations: Vec<NamespaceObservation>,
    reads: BTreeSet<StateRecordKey>,
    namespaces: BTreeSet<RevisionKey>,
}

impl ParseTransaction {
    pub(crate) fn enable_read_observations(&self) -> Result<(), StateError> {
        let mut parse = self.lock()?;
        parse.ensure_open()?;
        parse.observations.enabled = true;
        Ok(())
    }

    pub(crate) fn observation_checkpoint(&self) -> Result<StateObservationCheckpoint, StateError> {
        let parse = self.lock()?;
        parse.ensure_open()?;
        Ok(StateObservationCheckpoint {
            transaction_id: parse.transaction_id.clone(),
            first: parse.observations.entries.len(),
            mutations: parse.observations.mutation_attempts,
            failures: parse.observations.failures,
        })
    }
}

impl InvocationTransaction {
    pub(crate) fn observe_reads(&mut self) {
        self.track_reads = true;
    }

    pub(crate) fn read_certificate(
        &self,
        before: &StateObservationCheckpoint,
    ) -> Result<Option<StateReadCertificate>, StateError> {
        let parse = self.parse.lock().map_err(|_| StateError::Internal {
            message: "parse transaction mutex was poisoned".into(),
        })?;
        parse.ensure_open()?;
        if !Arc::ptr_eq(&parse.transaction_id, &before.transaction_id) {
            return Err(StateError::ForeignSavepoint);
        }
        if !self.track_reads
            || !parse.observations.enabled
            || parse.observations.exhausted
            || before.mutations != parse.observations.mutation_attempts
            || before.failures != parse.observations.failures
            || !self.writes.is_empty()
            || !self.write_set.is_empty()
        {
            return Ok(None);
        }
        Ok(Some(StateReadCertificate {
            component: self.component_id.clone(),
            observations: parse.observations.entries[before.first..].to_vec(),
            reads: self.reads.clone(),
            namespaces: self.observed_revisions.keys().cloned().collect(),
        }))
    }

    /// Validate and install fresh invocation reads under one store lock. The
    /// ordinary output validation still decides whether to commit or roll back.
    pub(crate) fn replay_read_certificate(
        &mut self,
        certificate: &StateReadCertificate,
    ) -> Result<bool, StateError> {
        if certificate.component != self.component_id
            || !self.writes.is_empty()
            || !self.reads.is_empty()
            || !self.observed_revisions.is_empty()
        {
            return Ok(false);
        }
        let mut parse = self.parse.lock().map_err(|_| StateError::Internal {
            message: "parse transaction mutex was poisoned".into(),
        })?;
        parse.ensure_open()?;
        let store_handle = parse.store.clone();
        let mut store = store_handle.lock()?;
        for observation in &certificate.observations {
            let Ok((namespace, _)) = store.namespaces.resolve(
                &observation.component,
                observation.key.namespace.visibility(),
                observation.key.namespace.name(),
                false,
            ) else {
                return Ok(false);
            };
            if namespace != observation.key.namespace {
                return Ok(false);
            }
            let mut values = if matches!(
                observation.key.scope,
                StateScope::Invocation | StateScope::Parse
            ) {
                BTreeMap::new()
            } else {
                store.committed_namespace(
                    &parse.project_uri,
                    &parse.document_id,
                    observation.key.scope,
                    &namespace,
                )?
            };
            for (address, value) in parse.writes.iter() {
                if address.scope == observation.key.scope && address.namespace == namespace {
                    apply_overlay(&mut values, &address.key, value);
                }
            }
            if values != observation.values {
                return Ok(false);
            }
        }
        for key in &certificate.namespaces {
            let revision = parse.revision(&mut store, key)?;
            self.observed_revisions.insert(key.clone(), revision);
        }
        self.reads.extend(certificate.reads.iter().cloned());
        for observation in &certificate.observations {
            parse.observations.read(
                &observation.component,
                observation.key.clone(),
                &observation.values,
            );
        }
        Ok(true)
    }

    pub(super) fn note_read_failure(&self) -> Result<(), StateError> {
        if self.track_reads {
            let mut parse = self.parse.lock().map_err(|_| StateError::Internal {
                message: "parse transaction mutex was poisoned".into(),
            })?;
            parse.observations.failures = parse.observations.failures.saturating_add(1);
        }
        Ok(())
    }

    pub(super) fn note_mutation_attempt(&self) -> Result<(), StateError> {
        if self.track_reads {
            let mut parse = self.parse.lock().map_err(|_| StateError::Internal {
                message: "parse transaction mutex was poisoned".into(),
            })?;
            parse.observations.mutation_attempts =
                parse.observations.mutation_attempts.saturating_add(1);
        }
        Ok(())
    }
}
