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
    retained_bytes: usize,
}

impl BodyReadCertificate {
    /// Payload bytes plus conservative per-record collection/struct allowance.
    pub(crate) fn retained_bytes(&self) -> usize { self.retained_bytes }

    /// Prefix receipts may change only outside the complete body read footprint.
    /// Namespace observations include negative and losing reads, and rereads of
    /// prefix keys which do not appear in the adopted set difference.
    fn prefix_reads_compatible(&self, current: &BTreeSet<StateRecordKey>) -> bool {
        if current == &self.prefix_reads { return true; }
        let observed = self.observations.iter().map(|entry| (
            entry.key.scope, entry.key.namespace.visibility(),
            entry.key.namespace.owner(), entry.key.namespace.name(),
        )).collect::<BTreeSet<_>>();
        self.prefix_reads.symmetric_difference(current).all(|key| !observed.contains(&(
            key.scope, key.visibility, key.owner.as_deref(), key.namespace.as_str(),
        )))
    }

    /// A set delta is bookkeeping, not a complete read footprint: a body may
    /// reread an existing prefix record. Keep observation-namespace overlap too.
    #[cfg(test)]
    pub(crate) fn compare_fresh_read_identity(&self,current:&Self)->serde_json::Value {
        let work=self.prefix_reads.len().saturating_add(self.adopted_reads.len())
            .saturating_add(current.prefix_reads.len()).saturating_add(current.adopted_reads.len());
        if work>16384 || self.observations.len().saturating_add(current.observations.len())>128 {
            return serde_json::json!({"blocked":"identity_work_limit"});
        }
        let changed=self.prefix_reads.symmetric_difference(&current.prefix_reads)
            .map(|key|(key.scope,key.owner.as_deref(),key.namespace.as_str())).collect::<BTreeSet<_>>();
        let old_observed=self.observations.iter().map(|v|(v.key.scope,v.key.namespace.owner(),v.key.namespace.name())).collect::<BTreeSet<_>>();
        let new_observed=current.observations.iter().map(|v|(v.key.scope,v.key.namespace.owner(),v.key.namespace.name())).collect::<BTreeSet<_>>();
        serde_json::json!({"prefixReadsEqual":self.prefix_reads==current.prefix_reads,
            "oldAddedReads":self.adopted_reads.difference(&self.prefix_reads).count(),
            "freshAddedReads":current.adopted_reads.difference(&current.prefix_reads).count(),
            "addedReadsEqual":self.adopted_reads.difference(&self.prefix_reads).eq(current.adopted_reads.difference(&current.prefix_reads)),
            "addedAdoptedNamespacesEqual":self.adopted_namespaces.difference(&self.prefix_adopted_namespaces)
                .eq(current.adopted_namespaces.difference(&current.prefix_adopted_namespaces)),
            "changedPrefixNamespaces":changed.len(),"oldBodyObservationOverlap":!changed.is_disjoint(&old_observed),
            "freshBodyObservationOverlap":!changed.is_disjoint(&new_observed),"observedNamespaceIdentitiesEqual":old_observed==new_observed,
            "scope":"identity comparison only; set difference is not the full body read footprint"})
    }
}

fn dependencies_size(set:&StateReadWriteSet) -> usize {
    let records=set.reads.iter().chain(&set.writes).fold(0usize,|n,key|
        n.saturating_add(192).saturating_add(key.key.len()).saturating_add(key.namespace.len())
            .saturating_add(key.owner.as_ref().map_or(0,String::len)));
    set.namespace_revisions.keys().fold(records,|n,key| n.saturating_add(192)
        .saturating_add(key.namespace.len()).saturating_add(key.owner.as_ref().map_or(0,String::len)))
}

impl ParseTransaction {
    /// Test-only explanation, never admission authority. Cheap prefix checks
    /// cover every proposal; a few requested details borrow values (no payload
    /// cloning) and stop before durable scopes, excessive work or byte reads.
    #[cfg(test)]
    pub(crate) fn body_read_refusal_profile(&self, certificate:&BodyReadCertificate, detail:bool) -> Result<serde_json::Value,StateError> {
        use sha2::{Digest,Sha256};
        let parse=self.lock()?;parse.ensure_open()?;
        let reads_equal=parse.read_write_set.reads==certificate.prefix_reads;
        let reads=certificate.prefix_reads_compatible(&parse.read_write_set.reads);
        let current=parse.current_revisions.keys().eq(certificate.prefix_namespaces.iter());
        let adopted=parse.read_write_set.namespace_revisions.keys().eq(certificate.prefix_adopted_namespaces.iter());
        let mut changed_namespaces=BTreeMap::new();let mut identity_blocked=false;
        if !reads_equal && parse.read_write_set.reads.len().saturating_add(certificate.prefix_reads.len())<=8192 {
            for (side,keys) in [("historicalOnly",certificate.prefix_reads.difference(&parse.read_write_set.reads)),
                ("currentOnly",parse.read_write_set.reads.difference(&certificate.prefix_reads))] {
                for key in keys {
                    if key.namespace.len().saturating_add(key.owner.as_ref().map_or(0,String::len))>1024 {identity_blocked=true;break;}
                    let identity=format!("{:?}|{}|{:x}",key.scope,key.owner.as_ref().map(|v|format!("{:x}",Sha256::digest(v))).unwrap_or_default(),Sha256::digest(&key.namespace));
                    if !changed_namespaces.contains_key(&identity)&&changed_namespaces.len()>=16 {identity_blocked=true;break;}
                    *changed_namespaces.entry(identity).or_insert_with(||BTreeMap::<&str,u64>::new()).entry(side).or_default()+=1;
                }
                if identity_blocked {break;}
            }
        } else if !reads_equal {identity_blocked=true;}
        let prefix_reason=if !parse.observations.enabled {Some("disabled")}
            else if parse.observations.exhausted {Some("exhausted")}
            else if !reads {Some("prefix_reads")}
            else if !current {Some("prefix_current_namespaces")}
            else if !adopted {Some("prefix_adopted_namespaces")} else {None};
        let mut rows=Vec::new();let mut work=0usize;let mut bytes=0usize;
        let mut observation_reason=None;let mut blocked=None;
        let mut prefix_identity_shapes_equal=None;
        let mut prefix_owners_equal=None;let mut prefix_scopes_equal=None;let mut prefix_names_equal=None;
        if detail && parse.read_write_set.reads.len().saturating_add(certificate.prefix_reads.len())<=4096 {
            fn shape(keys:&BTreeSet<StateRecordKey>)->BTreeSet<(StateScope,Option<&str>,&str)> {
                keys.iter().map(|key|(key.scope,key.owner.as_deref(),key.namespace.as_str())).collect()
            }
            // Namespace/owner/scope shapes only. Never log record keys.
            let current=shape(&parse.read_write_set.reads);let expected=shape(&certificate.prefix_reads);
            prefix_identity_shapes_equal=Some(current==expected);
            prefix_owners_equal=Some(current.iter().map(|(_,owner,_)|owner).collect::<BTreeSet<_>>()
                ==expected.iter().map(|(_,owner,_)|owner).collect::<BTreeSet<_>>());
            prefix_scopes_equal=Some(current.iter().map(|(scope,_,_)|scope).collect::<BTreeSet<_>>()
                ==expected.iter().map(|(scope,_,_)|scope).collect::<BTreeSet<_>>());
            prefix_names_equal=Some(current.iter().map(|(_,_,name)|name).collect::<BTreeSet<_>>()
                ==expected.iter().map(|(_,_,name)|name).collect::<BTreeSet<_>>());
        }
        if detail {
            let store=parse.store.lock()?;
            for observation in &certificate.observations {
                if rows.len()>=64 {blocked=Some("observation_limit");break;}
                if observation.key.scope!=StateScope::Parse {blocked=Some("non_parse_scope");break;}
                let resolved=store.namespaces.resolve(&observation.component,
                    observation.key.namespace.visibility(),observation.key.namespace.name(),false);
                let permission=match resolved {
                    Ok((namespace,_)) if namespace==observation.key.namespace=>None,
                    Ok(_)=>Some("owner_namespace_identity"),
                    Err(StateError::AccessDenied{..})=>Some("permission_denied"),
                    Err(StateError::UnknownNamespace{..})=>Some("reference_missing"),
                    Err(_)=>Some("namespace_lookup_error"),
                };
                let mut values=BTreeMap::<&str,&StateValue>::new();
                if permission.is_none() {
                    for (address,value) in parse.writes.iter() {
                        work+=1;if work>16384 {blocked=Some("overlay_work_limit");break;}
                        if address.scope==StateScope::Parse && address.namespace==observation.key.namespace {
                            if let Some(value)=value.as_ref() {values.insert(address.key.as_str(),value);}
                            else {values.remove(address.key.as_str());}
                            if values.len()>1024 {blocked=Some("record_limit");break;}
                        }
                    }
                }
                if blocked.is_some() {break;}
                let keys_equal=values.keys().copied().eq(observation.values.keys().map(String::as_str));
                let mut schemas_equal=true;let mut encodings_equal=true;let mut stored_bytes_equal=true;
                if keys_equal && permission.is_none() {
                    for ((key,value),expected) in values.iter().zip(observation.values.values()) {
                        bytes=bytes.saturating_add(key.len()).saturating_add(value.stored_size()).saturating_add(expected.stored_size());
                        if bytes>1024*1024 {blocked=Some("comparison_byte_limit");break;}
                        schemas_equal &= value.schema_id==expected.schema_id;
                        encodings_equal &= value.encoding==expected.encoding;
                        stored_bytes_equal &= value.bytes==expected.bytes;
                    }
                }
                if blocked.is_some() {break;}
                let reason=permission.or(if !keys_equal {Some("record_presence")}
                    else if !schemas_equal {Some("schema_changed")}
                    else if !encodings_equal {Some("encoding_changed")}
                    else if !stored_bytes_equal {Some("stored_bytes_changed")} else {None});
                if observation_reason.is_none() {observation_reason=reason;}
                rows.push(serde_json::json!({"scope":"Parse",
                    "namespaceSha256":format!("{:x}",Sha256::digest(observation.key.namespace.name())),
                    "ownerSha256":observation.key.namespace.owner().map(|owner|format!("{:x}",Sha256::digest(owner))),
                    "reason":reason,"keysEqual":keys_equal,"valueComparisonPerformed":keys_equal&&permission.is_none(),
                    "schemasEqual":(keys_equal&&permission.is_none()).then_some(schemas_equal),
                    "encodingsEqual":(keys_equal&&permission.is_none()).then_some(encodings_equal),
                    "storedBytesEqual":(keys_equal&&permission.is_none()).then_some(stored_bytes_equal),
                    "expectedRecords":observation.values.len(),"currentRecords":values.len()}));
            }
        }
        Ok(serde_json::json!({"firstPrefixFailure":prefix_reason,
            "prefixReadsEqual":reads_equal,"prefixReadsCompatible":reads,"prefixCurrentNamespacesEqual":current,"prefixAdoptedNamespacesEqual":adopted,
            "prefixChangedNamespaces":changed_namespaces,"prefixIdentityBudgetBlocked":identity_blocked,
            "expectedPrefixReads":certificate.prefix_reads.len(),"currentPrefixReads":parse.read_write_set.reads.len(),
            "prefixIdentityShapesEqual":prefix_identity_shapes_equal,"detailRequested":detail,
            "prefixOwnersEqual":prefix_owners_equal,"prefixScopesEqual":prefix_scopes_equal,"prefixNamespaceNamesEqual":prefix_names_equal,
            "observationFailure":observation_reason,"detailBlocked":blocked,"observations":rows,
            "allObservationsChecked":detail&&blocked.is_none(),"work":work,"comparedBytes":bytes,
            "limits":{"samplesAtCaller":8,"prefixKeys":4096,"observations":64,"overlayWork":16384,"records":1024,"comparedBytes":1048576}}))
    }

    /// Diagnostic admission predicate only. Parse-local comparisons cannot load
    /// durable data, update revisions, apply reads, or adopt observation history.
    #[cfg(test)]
    pub(crate) fn body_shadow_check_reads(&self, certificate:&BodyReadCertificate) -> Result<serde_json::Value,StateError> {
        use sha2::Digest;
        let parse=self.lock()?;parse.ensure_open()?;
        let prefix=certificate.prefix_reads_compatible(&parse.read_write_set.reads)
            && parse.current_revisions.keys().eq(certificate.prefix_namespaces.iter())
            && parse.read_write_set.namespace_revisions.keys().eq(certificate.prefix_adopted_namespaces.iter());
        let store=parse.store.lock()?;
        let mut rows=Vec::new();let mut all_match=true;let mut blocked=false;
        for observation in &certificate.observations {
            if rows.len()>=64 {blocked=true;break;}
            if observation.key.scope!=StateScope::Parse {blocked=true;break;}
            let namespace=store.namespaces.resolve(&observation.component,
                observation.key.namespace.visibility(),observation.key.namespace.name(),false).ok().map(|v|v.0);
            let permission=namespace.as_ref()==Some(&observation.key.namespace);
            let mut values=BTreeMap::new();
            if permission {
                for (index,(address,value)) in parse.writes.iter().enumerate() {
                    if index>=4096 {blocked=true;break;}
                    if address.scope==StateScope::Parse && address.namespace==observation.key.namespace {
                        apply_overlay(&mut values,&address.key,value);
                    }
                }
            }
            let matches=permission && values==observation.values;all_match &= matches;
            rows.push(serde_json::json!({"scope":"Parse","namespaceSha256":format!("{:x}",sha2::Sha256::digest(observation.key.namespace.name())),
                "permissionMatches":permission,"valuesEqual":matches,"keysEqual":values.keys().eq(observation.values.keys()),
                "expectedRecords":observation.values.len(),"currentRecords":values.len()}));
        }
        Ok(serde_json::json!({"status":if blocked || !parse.observations.enabled || parse.observations.exhausted {"Blocked"} else if prefix&&all_match {"Pass"} else {"Fail"},
            "mode":"predicate-only, no replay/import/adoption","prefixMatches":prefix,
            "enabled":parse.observations.enabled,"exhausted":parse.observations.exhausted,
            "observations":rows,"allValuesMatch":all_match,"otherScopeOrBudgetBlocked":blocked}))
    }

    /// Counts only; never exposes namespace names, record keys or stored values.
    #[cfg(test)]
    pub(crate) fn body_shadow_status(&self, before:&BodyObservationCheckpoint) -> Result<serde_json::Value,StateError> {
        let parse=self.lock()?;parse.ensure_open()?;
        if !Arc::ptr_eq(&parse.transaction_id,&before.state.transaction_id) {return Err(StateError::ForeignSavepoint);}
        let mut scopes=BTreeMap::<String,usize>::new();
        for observation in &parse.observations.entries[before.observation.first..] {
            *scopes.entry(format!("{:?}",observation.key.scope)).or_default()+=1;
        }
        Ok(serde_json::json!({"enabled":parse.observations.enabled,"exhausted":parse.observations.exhausted,
            "mutationAttempts":parse.observations.mutation_attempts.saturating_sub(before.observation.mutations),
            "readFailures":parse.observations.failures.saturating_sub(before.observation.failures),
            "writesEqual":parse.writes==before.state.writes,
            "writeSetEqual":parse.read_write_set.writes==before.state.read_write_set.writes,
            "overlayEqual":parse.overlay_revision==before.state.overlay_revision,
            "observedScopes":scopes}))
    }

    pub(crate) fn body_observation_checkpoint(&self) -> Result<BodyObservationCheckpoint, StateError> {
        // Capture observations, the overlay and the deferred-work generation
        // under one lock; no deferred read may fall between these boundaries.
        let mut parse = self.lock()?;
        parse.ensure_open()?;
        Ok(BodyObservationCheckpoint {
            observation: StateObservationCheckpoint {
                transaction_id: parse.transaction_id.clone(),
                first: parse.observations.entries.len(),
                mutations: parse.observations.mutation_attempts,
                failures: parse.observations.failures,
            },
            state: parse.savepoint()?,
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
            || parse.overlay_revision != before.state.overlay_revision
            || !before.state.applied_deltas.is_subset(&parse.applied_deltas)
            // Dependencies captured before this body must not be imported by
            // a later-created delta, even in a losing/rolled-back candidate.
            || parse.observations.dependency_origins[before.state.first_dependency_import..].iter()
                .any(|origin| *origin < before.state.origin)
            || !before.state.current_revisions.iter().all(|(key,value)| parse.current_revisions.get(key) == Some(value))
            || !before.state.base_revisions.iter().all(|(key,value)| parse.base_revisions.get(key) == Some(value))
            || !before.state.read_write_set.reads.is_subset(&parse.read_write_set.reads)
            || !before.state.current_revisions.keys().all(|key| parse.current_revisions.contains_key(key))
            || !before.state.read_write_set.namespace_revisions.keys().all(|key| parse.read_write_set.namespace_revisions.contains_key(key))
        {
            return Ok(None);
        }
        // Bound the owned certificate before cloning any prefix or namespace.
        let mut retained_bytes=dependencies_size(&before.state.read_write_set)
            .saturating_add(dependencies_size(&parse.read_write_set));
        for key in before.state.current_revisions.keys().chain(parse.current_revisions.keys()) {
            retained_bytes=retained_bytes.saturating_add(192).saturating_add(key.namespace.name().len())
                .saturating_add(key.namespace.owner().map_or(0,str::len));
        }
        for observation in &parse.observations.entries[observation.first..] {
            retained_bytes=retained_bytes.saturating_add(namespace_size(&observation.values))
                .saturating_add(observation.values.len().saturating_mul(192))
                .saturating_add(192).saturating_add(observation.component.len())
                .saturating_add(observation.key.namespace.name().len())
                .saturating_add(observation.key.namespace.owner().map_or(0,str::len));
        }
        if retained_bytes>MAX_OBSERVED_BYTES { return Ok(None); }
        Ok(Some(BodyReadCertificate {
            retained_bytes,
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
        if !parse.observations.enabled || parse.observations.exhausted
            || !certificate.prefix_reads_compatible(&parse.read_write_set.reads)
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
            if certificate.adopted_namespaces.contains(&public)
                && !parse.read_write_set.namespace_revisions.contains_key(&public)
            {
                Arc::make_mut(&mut parse.read_write_set)
                    .namespace_revisions.entry(public).or_insert(revision);
            }
        }
        // Keep the fresh prefix, including current preparation/receipt keys.
        // Only selected body reads are adopted; never copy the historical
        // prefix, or adopt reads from rejected speculative candidates.
        if certificate.adopted_reads.difference(&certificate.prefix_reads)
            .any(|key| !parse.read_write_set.reads.contains(key))
        {
            Arc::make_mut(&mut parse.read_write_set).reads.extend(
                certificate.adopted_reads.difference(&certificate.prefix_reads).cloned(),
            );
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
    fn prefix_receipt(parse: &ParseTransaction, key: &str) {
        let mut call = parse.begin_invocation(OWNER).unwrap();
        call.observe_reads();
        call.get(StateScope::Parse, NamespaceVisibility::Private, "losing", key).unwrap();
        call.commit().unwrap();
    }
    #[test]
    fn body_certificate_merges_only_body_reads_into_the_current_prefix() {
        let store = store();
        let first = parse(&store, 1);
        prefix_receipt(&first, "old-receipt");
        let before = first.body_observation_checkpoint().unwrap();
        get(&first, "selected", true);
        get(&first, "rejected", false);
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.cancel().unwrap();

        for revision in 2..6 {
            let receipt = format!("current-receipt-{revision}");
            let current = parse(&store, revision);
            prefix_receipt(&current, &receipt);
            let prefix = current.savepoint().unwrap();
            let expected_prefix = current.read_write_set().unwrap();
            let report = current.body_read_refusal_profile(&certificate, true).unwrap();
            assert_eq!(report["prefixReadsEqual"], false);
            assert_eq!(report["prefixReadsCompatible"], true);
            assert_eq!(current.body_shadow_check_reads(&certificate).unwrap()["status"], "Pass");
            assert!(current.replay_body_read_certificate(&certificate).unwrap());
            let actual = current.read_write_set().unwrap();
            assert!(expected_prefix.reads.is_subset(&actual.reads));
            assert!(!actual.reads.iter().any(|read| matches!(read.key.as_str(), "old-receipt" | "rejected")));
            current.rollback_to(&prefix).unwrap();
            assert_eq!(current.read_write_set().unwrap(), expected_prefix);
            get(&current, "selected", true);
            get(&current, "rejected", false);
            assert_eq!(current.read_write_set().unwrap(), actual, "cold dependencies in the current revision");
            current.cancel().unwrap();
        }
    }
    #[test]
    fn body_certificate_namespace_overlap_includes_rereads_and_losing_empty_scans() {
        for reread in [false, true] {
            let store = store();
            let first = parse(&store, 1);
            prefix_receipt(&first, "old");
            let before = first.body_observation_checkpoint().unwrap();
            if reread {
                prefix_receipt(&first, "old");
            } else {
                let mut losing = first.begin_invocation(OWNER).unwrap();
                losing.observe_reads();
                assert!(losing.scan_prefix(StateScope::Parse, NamespaceVisibility::Private, "losing", "missing", 1).unwrap().is_empty());
                losing.rollback();
            }
            let certificate = first.body_read_certificate(&before).unwrap().unwrap();
            assert!(certificate.adopted_reads.difference(&certificate.prefix_reads).next().is_none());
            first.cancel().unwrap();
            let current = parse(&store, 2);
            prefix_receipt(&current, "new");
            let prefix = current.read_write_set().unwrap();
            assert!(!current.replay_body_read_certificate(&certificate).unwrap());
            assert_eq!(current.read_write_set().unwrap(), prefix);
            current.cancel().unwrap();
        }
    }
    #[test]
    fn body_certificate_prefix_compatibility_uses_scope_visibility_and_owner() {
        let store = store();
        let first = parse(&store, 1);
        let before = first.body_observation_checkpoint().unwrap();
        get(&first, "value", false);
        let mut certificate = first.body_read_certificate(&before).unwrap().unwrap();
        let mut key = StateRecordKey { scope: StateScope::Parse, visibility: NamespaceVisibility::Private,
            owner: Some(OWNER.into()), namespace: NAMESPACE.into(), key: "old".into() };
        for distinct in ["same", "scope", "owner", "visibility", "name"] {
            key.scope = StateScope::Parse; key.visibility = NamespaceVisibility::Private;
            key.owner = Some(OWNER.into()); key.namespace = NAMESPACE.into(); key.key = "old".into();
            match distinct {
                "scope" => key.scope = StateScope::Project,
                "owner" => key.owner = Some("test.other".into()),
                "visibility" => { key.visibility = NamespaceVisibility::Shared; key.owner = None; },
                "name" => key.namespace = "other".into(),
                _ => {},
            }
            certificate.prefix_reads = BTreeSet::from([key.clone()]);
            key.key = "current".into();
            assert_eq!(certificate.prefix_reads_compatible(&BTreeSet::from([key.clone()])), distinct != "same", "{distinct}");
        }
        first.cancel().unwrap();
    }
    #[test]
    fn body_certificate_rejects_pre_body_deferred_reads_including_nested_deltas() {
        for (nested, rolled_back) in [(false, false), (true, false), (false, true), (true, true)] {
            let store = store();
            let current = parse(&store, 1);
            let prefix = current.savepoint().unwrap();
            get(&current, "deferred-read", true);
            let old = current.defer_since(&prefix).unwrap();
            let before = current.body_observation_checkpoint().unwrap();
            assert!(current.apply_delta(&old).unwrap());
            if nested {
                let outer = current.defer_since(&prefix).unwrap();
                assert!(current.apply_delta(&outer).unwrap());
            }
            if rolled_back { current.rollback_to(&prefix).unwrap(); }
            assert!(current.body_read_certificate(&before).unwrap().is_none(), "{nested}/{rolled_back}");
            current.cancel().unwrap();
        }
    }
    #[test]
    fn body_certificate_rejects_new_deltas_from_savepoints_crossing_the_body_boundary() {
        for nested in [false, true] {
            let store = store();
            let current = parse(&store, 1);
            let old = current.savepoint().unwrap();
            get(&current, "pre-body-read", true);
            let before = current.body_observation_checkpoint().unwrap();
            let late = current.defer_since(&old).unwrap();
            assert!(late.id > before.state.origin);
            assert!(late.origin < before.state.origin, "creation time is not the dependency origin");
            assert!(current.apply_delta(&late).unwrap());
            if nested {
                let outer = current.defer_since(&before.state).unwrap();
                assert_eq!(outer.origin, old.origin, "wrapping a late delta must not erase its older dependencies");
                assert!(current.apply_delta(&outer).unwrap());
            }
            assert!(current.body_read_certificate(&before).unwrap().is_none(), "{nested}");
            current.cancel().unwrap();
        }
    }
    #[test]
    fn body_certificate_rejects_pre_body_snapshot_dependency_imports_after_rollback() {
        for rolled_back in [false, true] {
            let store = store();
            let current = parse(&store, 1);
            get(&current, "prefix", true);
            let prefix = current.savepoint().unwrap();
            get(&current, "earlier-branch-read", true);
            let earlier = current.savepoint().unwrap();
            current.rollback_to(&prefix).unwrap();
            let before = current.body_observation_checkpoint().unwrap();
            current.rollback_to(&earlier).unwrap();
            if rolled_back { current.rollback_to(&prefix).unwrap(); }
            assert!(current.body_read_certificate(&before).unwrap().is_none(), "{rolled_back}");
            current.cancel().unwrap();
        }
    }
    #[test]
    fn body_certificate_allows_body_local_snapshot_imports_and_dependency_discard() {
        for restore_body_snapshot in [false, true] {
            let store = store();
            let current = parse(&store, 1);
            get(&current, "prefix", true);
            let prefix = current.savepoint().unwrap();
            let before = current.body_observation_checkpoint().unwrap();
            let history = current.lock().unwrap().observations.dependency_origins.len();
            get(&current, "body", true);
            let body_snapshot = current.savepoint().unwrap();
            current.rollback_to(&prefix).unwrap();
            assert_eq!(current.lock().unwrap().observations.dependency_origins.len(), history,
                "dropping dependencies must not attribute reads to an older prefix");
            if restore_body_snapshot { current.rollback_to(&body_snapshot).unwrap(); }
            assert!(current.body_read_certificate(&before).unwrap().is_some(), "{restore_body_snapshot}");
            current.cancel().unwrap();
        }
    }
    #[test]
    fn body_certificate_allows_body_local_deferred_reads_and_existing_prefix_deltas() {
        let store = store();
        let first = parse(&store, 1);
        let prefix = first.savepoint().unwrap();
        prefix_receipt(&first, "old");
        let prefix_delta = first.defer_since(&prefix).unwrap();
        first.apply_delta(&prefix_delta).unwrap();
        let before = first.body_observation_checkpoint().unwrap();
        assert!(!first.apply_delta(&prefix_delta).unwrap(), "already-applied prefix delta is a no-op");
        let point = first.savepoint().unwrap();
        get(&first, "selected", true);
        let body_delta = first.defer_since(&point).unwrap();
        first.apply_delta(&body_delta).unwrap();
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.cancel().unwrap();
        let current = parse(&store, 2);
        prefix_receipt(&current, "new");
        let prefix = current.read_write_set().unwrap();
        assert!(current.replay_body_read_certificate(&certificate).unwrap());
        assert!(prefix.reads.is_subset(&current.read_write_set().unwrap().reads));
        assert!(current.lock().unwrap().applied_deltas.is_empty(), "historical delta receipts grant no current authority");
        current.cancel().unwrap();
    }
    #[test]
    fn body_certificate_delta_observation_limit_fails_closed() {
        let store = store();
        let current = parse(&store, 1);
        let point = current.savepoint().unwrap();
        let before = current.body_observation_checkpoint().unwrap();
        let delta = current.defer_since(&point).unwrap();
        current.lock().unwrap().observations.dependency_origins = vec![delta.origin; MAX_OBSERVATIONS];
        assert!(current.apply_delta(&delta).unwrap());
        assert!(current.lock().unwrap().observations.exhausted);
        assert!(current.body_read_certificate(&before).unwrap().is_none());
        current.cancel().unwrap();
    }
    #[test]
    fn body_certificate_savepoint_origin_clock_never_wraps_or_rolls_back() {
        let store = store();
        let current = parse(&store, 1);
        let point = current.savepoint().unwrap();
        let later = current.savepoint().unwrap();
        current.rollback_to(&point).unwrap();
        let after = current.savepoint().unwrap();
        assert!(point.origin < later.origin && later.origin < after.origin);
        current.lock().unwrap().next_delta_id = u64::MAX;
        assert!(current.savepoint().is_err());
        assert!(current.body_observation_checkpoint().is_err());
        assert_eq!(current.lock().unwrap().next_delta_id, u64::MAX);
        current.cancel().unwrap();
    }
    #[test]
    fn body_certificate_disjoint_prefix_still_checks_values_and_permissions() {
        let store = store();
        let first = parse(&store, 1);
        prefix_receipt(&first, "old");
        let before = first.body_observation_checkpoint().unwrap();
        get(&first, "missing", false);
        let certificate = first.body_read_certificate(&before).unwrap().unwrap();
        first.cancel().unwrap();
        for changed_value in [false, true] {
            let current = parse(&store, if changed_value { 3 } else { 2 });
            prefix_receipt(&current, "new");
            let mut cert = certificate.clone();
            if changed_value {
                let mut write = current.begin_invocation(OWNER).unwrap();
                write.put(StateScope::Parse, NamespaceVisibility::Private, NAMESPACE, "missing",
                    StateValue::new("test.values", StateEncoding::Raw, b"now-present")).unwrap();
                write.commit().unwrap();
            } else { cert.observations[0].component = "test.unknown".into(); }
            let unchanged = current.read_write_set().unwrap();
            assert!(!current.replay_body_read_certificate(&cert).unwrap());
            assert_eq!(current.read_write_set().unwrap(), unchanged);
            current.cancel().unwrap();
        }
    }
    #[test]
    fn refusal_profile_distinguishes_prefix_identity_values_and_unobserved_limits_without_effects() {
        let store=store();
        store.register_component("test.other",&[NamespaceDeclaration::private(NAMESPACE,"test.values",1),
            NamespaceDeclaration::shared("protected","test.values",1,Vec::<String>::new(),Vec::<String>::new())]).unwrap();
        fn seed(parse:&ParseTransaction) {
            let mut call=parse.begin_invocation(OWNER).unwrap();
            call.put(StateScope::Parse,NamespaceVisibility::Private,NAMESPACE,"value",StateValue::new("test.values",StateEncoding::Raw,b"same")).unwrap();
            call.commit().unwrap();get(parse,"prefix",true);
        }
        let first=parse(&store,1);seed(&first);let point=first.body_observation_checkpoint().unwrap();
        get(&first,"value",true);let cert=first.body_read_certificate(&point).unwrap().unwrap();first.cancel().unwrap();
        let mut unrelated=cert.clone();let mut key=cert.prefix_reads.iter().next().unwrap().clone();
        key.namespace="unrelated".into();key.key="new".into();unrelated.prefix_reads.insert(key.clone());unrelated.adopted_reads.insert(key.clone());
        let identity=cert.compare_fresh_read_identity(&unrelated);assert_eq!(identity["addedReadsEqual"],true);assert_eq!(identity["oldBodyObservationOverlap"],false);
        key.namespace=NAMESPACE.into();let mut overlapping=cert.clone();overlapping.prefix_reads.insert(key.clone());overlapping.adopted_reads.insert(key);
        let identity=cert.compare_fresh_read_identity(&overlapping);assert_eq!(identity["addedReadsEqual"],true);
        assert_eq!(identity["oldBodyObservationOverlap"],true,"equal deltas cannot prove an unobserved reread irrelevant");
        for (index,case) in ["pass","prefix_reads","prefix_current_namespaces","prefix_adopted_namespaces","overlap",
            "reference_missing","owner_namespace_identity","permission_denied","record_presence","schema_changed",
            "encoding_changed","stored_bytes_changed","value_then_blocked","non_parse_scope","comparison_byte_limit","observation_limit","disabled","exhausted"].into_iter().enumerate() {
            let current=parse(&store,index as u64+2);seed(&current);let mut value=cert.clone();
            match case {
                "prefix_reads"|"overlap"=>{value.prefix_reads.clear();if case=="overlap" {value.prefix_namespaces.clear();value.prefix_adopted_namespaces.clear();}},
                "prefix_current_namespaces"=>value.prefix_namespaces.clear(),
                "prefix_adopted_namespaces"=>value.prefix_adopted_namespaces.clear(),
                "reference_missing"=>value.observations[0].component="test.missing".into(),
                "owner_namespace_identity"=>value.observations[0].component="test.other".into(),
                "permission_denied"=>value.observations[0].key.namespace=NamespaceKey::Shared{name:"protected".into()},
                "record_presence"=>{value.observations[0].values.clear();},
                "schema_changed"=>value.observations[0].values.get_mut("value").unwrap().schema_id="other".into(),
                "encoding_changed"=>value.observations[0].values.get_mut("value").unwrap().encoding=StateEncoding::Json,
                "stored_bytes_changed"=>value.observations[0].values.get_mut("value").unwrap().bytes=b"other".to_vec(),
                "value_then_blocked"=>{value.observations[0].values.get_mut("value").unwrap().bytes=b"other".to_vec();let mut later=value.observations[0].clone();later.key.scope=StateScope::Project;value.observations.push(later);},
                "non_parse_scope"=>value.observations[0].key.scope=StateScope::Project,
                "comparison_byte_limit"=>value.observations[0].values.get_mut("value").unwrap().bytes=vec![0;1048577],
                "observation_limit"=>value.observations=vec![value.observations[0].clone();65],
                "disabled"=>current.lock().unwrap().observations.enabled=false,
                "exhausted"=>current.lock().unwrap().observations.exhausted=true,
                "pass"=>{},_=>unreachable!(),
            }
            let before=current.body_observation_checkpoint().unwrap();let state=current.read_write_set().unwrap();
            let observations=current.body_shadow_status(&before).unwrap();
            let report=current.body_read_refusal_profile(&value,true).unwrap();
            let serialized=report.to_string();assert!(!serialized.contains("\"same\"")&&!serialized.contains("\"prefix\"")&&!serialized.contains("\"value\""));
            assert_eq!(current.read_write_set().unwrap(),state,"{case}");
            assert_eq!(current.body_shadow_status(&before).unwrap(),observations,"{case}");
            let expected=if case=="overlap" {"prefix_reads"} else if case=="value_then_blocked" {"stored_bytes_changed"} else {case};
            let actual=report["firstPrefixFailure"].as_str().or(report["observationFailure"].as_str()).or(report["detailBlocked"].as_str());
            assert_eq!(actual,if case=="pass" {None} else {Some(expected)},"{case}: {report}");
            assert_eq!(current.replay_body_read_certificate(&value).unwrap(),matches!(case,"pass"|"observation_limit"),"{case}");
            current.cancel().unwrap();
        }
    }
    #[test]
    fn shadow_read_admission_is_nonmutating_and_matches_actual_parse_scope_replay() {
        let store=store();let first=parse(&store,1);
        get(&first,"prefix",true);let checkpoint=first.body_observation_checkpoint().unwrap();
        get(&first,"selected",true);get(&first,"negative",false);
        let certificate=first.body_read_certificate(&checkpoint).unwrap().unwrap();first.cancel().unwrap();
        let second=parse(&store,2);get(&second,"prefix",true);
        let before=second.read_write_set().unwrap();let checkpoint=second.body_observation_checkpoint().unwrap();
        let report=second.body_shadow_check_reads(&certificate).unwrap();assert_eq!(report["status"],"Pass");
        assert_eq!(second.read_write_set().unwrap(),before);
        assert_eq!(second.body_shadow_status(&checkpoint).unwrap()["observedScopes"],serde_json::json!({}));
        assert!(second.replay_body_read_certificate(&certificate).unwrap(),"isolated unit replay after, never during, the predicate");
        second.cancel().unwrap();
        let third=parse(&store,3);get(&third,"prefix",true);
        let mut write=third.begin_invocation(OWNER).unwrap();
        write.put(StateScope::Parse,NamespaceVisibility::Private,NAMESPACE,"negative",StateValue::new("test.values",StateEncoding::Raw,b"changed")).unwrap();write.commit().unwrap();
        let before=third.read_write_set().unwrap();
        assert_eq!(third.body_shadow_check_reads(&certificate).unwrap()["status"],"Fail");
        assert_eq!(third.read_write_set().unwrap(),before);
        assert!(!third.replay_body_read_certificate(&certificate).unwrap());
        third.cancel().unwrap();
    }

    #[test]
    fn shadow_read_admission_rejects_prefix_permissions_and_unobserved_scopes_without_applying() {
        let store=store();let first=parse(&store,1);
        get(&first,"prefix",true);let before=first.body_observation_checkpoint().unwrap();get(&first,"body",true);
        let certificate=first.body_read_certificate(&before).unwrap().unwrap();first.cancel().unwrap();
        let second=parse(&store,2);get(&second,"different-prefix",true);
        let before=second.read_write_set().unwrap();let check=second.body_shadow_check_reads(&certificate).unwrap();
        assert_eq!(check["status"],"Fail");assert_eq!(check["prefixMatches"],false);assert_eq!(second.read_write_set().unwrap(),before);
        second.cancel().unwrap();
        let third=parse(&store,3);get(&third,"prefix",true);let before=third.read_write_set().unwrap();
        let mut inaccessible=certificate.clone();inaccessible.observations[0].component="unregistered.foreign".into();
        assert_eq!(third.body_shadow_check_reads(&inaccessible).unwrap()["status"],"Fail");
        let mut other_scope=certificate.clone();other_scope.observations[0].key.scope=StateScope::Project;
        assert_eq!(third.body_shadow_check_reads(&other_scope).unwrap()["status"],"Blocked");
        third.lock().unwrap().observations.exhausted=true;
        assert_eq!(third.body_shadow_check_reads(&certificate).unwrap()["status"],"Blocked");
        assert_eq!(third.read_write_set().unwrap(),before);third.cancel().unwrap();
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
    fn body_certificate_rejects_rollback_of_equal_prefix_writes_and_revision_receipts() {
        let store = store();
        let parse = parse(&store, 1);
        let put = || {
            let mut call = parse.begin_invocation(OWNER).unwrap();
            call.put(StateScope::Parse, NamespaceVisibility::Private, NAMESPACE, "x",
                StateValue::new("test.values", StateEncoding::Raw, b"same")).unwrap();
            call.commit().unwrap();
        };
        put();
        let earlier = parse.savepoint().unwrap();
        put();
        let before = parse.body_observation_checkpoint().unwrap();
        parse.rollback_to(&earlier).unwrap();
        assert!(parse.body_read_certificate(&before).unwrap().is_none());
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
