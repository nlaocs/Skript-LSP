//! Readonly dependencies of a complete native body. Observations include losing
//! candidates; only adopted reads are restored. No old transaction is replayed.
use super::*;

pub(crate) struct BodyObservationCheckpoint {
    observation: StateObservationCheckpoint,
    state: StateSavepoint,
}

#[derive(Debug, Clone)]
pub(crate) struct BodyReadCertificate {
    observations: Vec<NamespaceObservation>,
    prefix_reads: BTreeSet<StateRecordKey>,
    prefix_namespaces: BTreeSet<RevisionKey>,
    prefix_adopted_namespaces: BTreeSet<StateNamespaceKey>,
    adopted_reads: BTreeSet<StateRecordKey>,
    namespaces: BTreeSet<RevisionKey>,
    adopted_namespaces: BTreeSet<StateNamespaceKey>,
}

impl ParseTransaction {
    pub(crate) fn body_observation_checkpoint(&self) -> Result<BodyObservationCheckpoint, StateError> {
        Ok(BodyObservationCheckpoint {
            observation: self.observation_checkpoint()?,
            state: self.savepoint()?,
        })
    }

    pub(crate) fn body_read_certificate(
        &self,
        before: &BodyObservationCheckpoint,
    ) -> Result<Option<BodyReadCertificate>, StateError> {
        let parse = self.lock()?;
        parse.ensure_open()?;
        let observation = &before.observation;
        if !Arc::ptr_eq(&parse.transaction_id, &observation.transaction_id)
            || !Arc::ptr_eq(&parse.transaction_id, &before.state.transaction_id)
        {
            return Err(StateError::ForeignSavepoint);
        }
        if !parse.observations.enabled
            || parse.observations.exhausted
            || observation.mutations == u64::MAX
            || observation.failures == u64::MAX
            || observation.mutations != parse.observations.mutation_attempts
            || observation.failures != parse.observations.failures
            || parse.writes != before.state.writes
            || parse.read_write_set.writes != before.state.read_write_set.writes
            || !before.state.read_write_set.reads.is_subset(&parse.read_write_set.reads)
            || !before.state.current_revisions.keys().all(|key| parse.current_revisions.contains_key(key))
            || !before.state.read_write_set.namespace_revisions.keys().all(|key| parse.read_write_set.namespace_revisions.contains_key(key))
        {
            return Ok(None);
        }
        Ok(Some(BodyReadCertificate {
            observations: parse.observations.entries[observation.first..].to_vec(),
            prefix_reads: before.state.read_write_set.reads.clone(),
            prefix_namespaces: before.state.current_revisions.keys().cloned().collect(),
            prefix_adopted_namespaces: before.state.read_write_set.namespace_revisions.keys().cloned().collect(),
            adopted_reads: parse.read_write_set.reads.clone(),
            namespaces: parse.current_revisions.keys().cloned().collect(),
            adopted_namespaces: parse.read_write_set.namespace_revisions.keys().cloned().collect(),
        }))
    }

    /// Namespace values, permissions and the current overlay are checked before
    /// any adoption under the same locks. Current revision dependencies replace
    /// historical numbers, including reads of namespaces already in the prefix.
    pub(crate) fn replay_body_read_certificate(
        &self,
        certificate: &BodyReadCertificate,
    ) -> Result<bool, StateError> {
        let mut parse = self.lock()?;
        parse.ensure_open()?;
        if parse.read_write_set.reads != certificate.prefix_reads
            || !parse.current_revisions.keys().eq(certificate.prefix_namespaces.iter())
            || !parse.read_write_set.namespace_revisions.keys().eq(certificate.prefix_adopted_namespaces.iter())
        {
            return Ok(false);
        }
        let store_handle = parse.store.clone();
        let mut store = store_handle.lock()?;
        if !observations_match(&parse, &mut store, &certificate.observations)? {
            return Ok(false);
        }
        for key in &certificate.namespaces {
            let revision = parse.revision(&mut store, key)?;
            let public = key.public();
            if certificate.adopted_namespaces.contains(&public) {
                Arc::make_mut(&mut parse.read_write_set)
                    .namespace_revisions.entry(public).or_insert(revision);
            }
        }
        if parse.read_write_set.reads != certificate.adopted_reads {
            Arc::make_mut(&mut parse.read_write_set).reads = certificate.adopted_reads.clone();
        }
        // Preserve rejected/negative observations for an enclosing certified
        // scope, without adopting their speculative read/write dependencies.
        for observation in &certificate.observations {
            parse.observations.read(
                &observation.component, observation.key.clone(), &observation.values,
            );
        }
        Ok(true)
    }

    /// A resource from another parse or an unconsumed branch grants no authority.
    pub(crate) fn delta_is_currently_applied(&self, delta: &StateDelta) -> Result<bool, StateError> {
        let parse = self.lock()?;
        parse.ensure_open()?;
        Ok(Arc::ptr_eq(&parse.transaction_id, &delta.transaction_id)
            && parse.applied_deltas.contains(&delta.id))
    }

    pub(crate) fn owns_savepoint(&self, point: &StateSavepoint) -> Result<bool, StateError> {
        let parse = self.lock()?;
        parse.ensure_open()?;
        Ok(Arc::ptr_eq(&parse.transaction_id, &point.transaction_id))
    }
}

pub(super) fn observations_match(
    parse: &ParseTransactionInner,
    store: &mut StateStoreInner,
    observations: &[NamespaceObservation],
) -> Result<bool, StateError> {
    for observation in observations {
        let Ok((namespace, _)) = store.namespaces.resolve(
            &observation.component, observation.key.namespace.visibility(),
            observation.key.namespace.name(), false,
        ) else {
            return Ok(false);
        };
        if namespace != observation.key.namespace {
            return Ok(false);
        }
        let mut values = if matches!(observation.key.scope, StateScope::Invocation | StateScope::Parse) {
            BTreeMap::new()
        } else {
            store.committed_namespace(
                &parse.project_uri, &parse.document_id, observation.key.scope, &namespace,
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
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "test.body-reuse";
    const DOCUMENT: &str = "file:///public/body.sk";
    const NAMESPACE: &str = "values";
    fn store() -> StateStore {
        let store = StateStore::new(StateStoreConfig::default()).unwrap();
        store.register_component(OWNER, &[
            NamespaceDeclaration::private(NAMESPACE, "test.values", 1),
            NamespaceDeclaration::private("losing", "test.values", 1),
        ]).unwrap();
        store
    }
    fn parse(store: &StateStore, revision: u64) -> ParseTransaction {
        let parse = store.begin_parse("file:///public", DOCUMENT, revision).unwrap();
        parse.enable_read_observations().unwrap();
        parse
    }
    fn get(parse: &ParseTransaction, key: &str, adopted: bool) {
        let mut call = parse.begin_invocation(OWNER).unwrap();
        call.observe_reads();
        call.get(StateScope::Parse, NamespaceVisibility::Private, NAMESPACE, key).unwrap();
        if adopted { call.commit().unwrap(); } else { call.rollback(); }
    }
    #[test]
    fn body_certificate_tracks_losing_observations_but_only_adopts_selected_reads() {
        let store = store();
        let first = parse(&store, 1);
        get(&first, "prefix", true);
        let before = first.body_observation_checkpoint().unwrap();
        get(&first, "prefix", true);
        get(&first, "adopted", true);
        get(&first, "negative", false);
        let expected = first.read_write_set().unwrap();
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.cancel().unwrap();

        let second = parse(&store, 2);
        get(&second, "prefix", true);
        assert!(second.replay_body_read_certificate(&certificate).unwrap());
        assert_eq!(second.read_write_set().unwrap(), expected);
        assert!(!second.read_write_set().unwrap().reads.iter().any(|read| read.key == "negative"));
        second.cancel().unwrap();

        let third = parse(&store, 3);
        get(&third, "prefix", true);
        let mut write = third.begin_invocation(OWNER).unwrap();
        write.put(StateScope::Parse, NamespaceVisibility::Private, NAMESPACE, "negative",
            StateValue::new("test.values", StateEncoding::Raw, b"changed")).unwrap();
        write.commit().unwrap();
        let unchanged = third.read_write_set().unwrap();
        assert!(!third.replay_body_read_certificate(&certificate).unwrap());
        assert_eq!(third.read_write_set().unwrap(), unchanged);
    }
    #[test]
    fn body_certificate_does_not_adopt_a_losing_only_namespace() {
        let store = store();
        let first = parse(&store, 1);
        let before = first.body_observation_checkpoint().unwrap();
        get(&first, "accepted", true);
        let mut losing = first.begin_invocation(OWNER).unwrap();
        losing.observe_reads();
        losing.get(StateScope::Parse, NamespaceVisibility::Private, "losing", "absent").unwrap();
        losing.rollback();
        let expected = first.read_write_set().unwrap();
        assert!(!expected.namespace_revisions.keys().any(|key| key.namespace == "losing"));
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.cancel().unwrap();
        let second = parse(&store, 2);
        assert!(second.replay_body_read_certificate(&certificate).unwrap());
        assert_eq!(second.read_write_set().unwrap(), expected);
    }
    #[test]
    fn body_certificate_rejects_rolled_back_mutation_attempts_and_changed_prefix() {
        let store = store();
        let first = parse(&store, 1);
        let before = first.body_observation_checkpoint().unwrap();
        let mut call = first.begin_invocation(OWNER).unwrap();
        call.observe_reads();
        call.put(StateScope::Parse, NamespaceVisibility::Private, NAMESPACE, "x",
            StateValue::new("test.values", StateEncoding::Raw, b"value")).unwrap();
        call.rollback();
        assert!(first.body_read_certificate(&before).unwrap().is_none());
        first.cancel().unwrap();

        let second = parse(&store, 2);
        let before = second.body_observation_checkpoint().unwrap();
        get(&second, "x", true);
        let certificate = second.body_read_certificate(&before).unwrap().unwrap();
        second.cancel().unwrap();
        let third = parse(&store, 3);
        get(&third, "unrelated-prefix", true);
        let unchanged = third.read_write_set().unwrap();
        assert!(!third.replay_body_read_certificate(&certificate).unwrap());
        assert_eq!(third.read_write_set().unwrap(), unchanged);
    }
    #[test]
    fn body_certificate_rejects_foreign_checkpoints_failed_reads_and_exhaustion() {
        let store = store();
        let first = parse(&store, 1);
        let before = first.body_observation_checkpoint().unwrap();
        let mut call = first.begin_invocation(OWNER).unwrap();
        call.observe_reads();
        call.note_read_failure().unwrap();
        assert!(first.body_read_certificate(&before).unwrap().is_none());
        first.cancel().unwrap();
        let second = parse(&store, 2);
        assert!(matches!(second.body_read_certificate(&before), Err(StateError::ForeignSavepoint)));
        let before = second.body_observation_checkpoint().unwrap();
        second.lock().unwrap().observations.exhausted = true;
        assert!(second.body_read_certificate(&before).unwrap().is_none());
    }

    #[test]
    fn body_certificate_rejects_missing_prefix_empty_scan_and_prefix_rollback() {
        let store = store();
        let first = parse(&store, 1);
        let earlier = first.savepoint().unwrap();
        let mut scan = first.begin_invocation(OWNER).unwrap();
        scan.observe_reads();
        assert!(scan.scan_prefix(StateScope::Parse, NamespaceVisibility::Private, NAMESPACE, "", 1).unwrap().is_empty());
        scan.commit().unwrap();
        let before = first.body_observation_checkpoint().unwrap();
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.rollback_to(&earlier).unwrap();
        assert!(first.body_read_certificate(&before).unwrap().is_none());
        first.cancel().unwrap();
        let second = parse(&store, 2);
        assert!(!second.replay_body_read_certificate(&certificate).unwrap());
        assert_eq!(second.read_write_set().unwrap(), StateReadWriteSet::default());
    }

    #[test]
    fn body_certificate_recomputes_current_revision_for_equal_namespace_values() {
        let store = store();
        let write = |revision| {
            let parse = parse(&store, revision);
            let mut call = parse.begin_invocation(OWNER).unwrap();
            call.put(StateScope::Project, NamespaceVisibility::Private, NAMESPACE, "stable",
                StateValue::new("test.values", StateEncoding::Raw, b"value")).unwrap();
            call.commit().unwrap();
            parse.commit().unwrap();
        };
        write(1);
        let first = parse(&store, 2);
        let before = first.body_observation_checkpoint().unwrap();
        let mut call = first.begin_invocation(OWNER).unwrap();
        call.observe_reads();
        call.get(StateScope::Project, NamespaceVisibility::Private, NAMESPACE, "stable").unwrap();
        call.commit().unwrap();
        let old = first.read_write_set().unwrap();
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.cancel().unwrap();
        write(3);
        let current = parse(&store, 4);
        assert!(current.replay_body_read_certificate(&certificate).unwrap());
        let fresh = current.read_write_set().unwrap();
        assert_eq!(fresh.reads, old.reads);
        assert_ne!(fresh.namespace_revisions, old.namespace_revisions);
        assert!(fresh.namespace_revisions.values().all(|value| *value == 2));
    }
}
