//! Structural checks for the proposed mixed-registry storage evidence contract.
//!
//! A checked capture/claim is NOT an authenticated Java fact. In particular this
//! module cannot issue ResultType, metadata, a native seal or an origin token.
//! A future collector/source-model integration must authenticate these inputs
//! separately. Existing mixed snapshots lack them and remain unsupported.
use super::domain::{Class, Graph, RegistryGeometry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) mod observation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SourceIdentity {
    pub snapshot_id: String,
    pub source_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ScopeSubject {
    pub document: String,
    pub revision: u64,
    pub syntax_context: u64,
    pub expansion: Option<u64>,
    pub start: u64,
    pub end: u64,
}

impl ScopeSubject {
    pub(super) fn borrowed(&self) -> ScopeRef<'_> {
        ScopeRef { document:&self.document, revision:self.revision, syntax_context:self.syntax_context,
            expansion:self.expansion, start:self.start, end:self.end }
    }

    pub(super) fn covers(&self, current: &Self) -> bool {
        self.borrowed().covers(current.borrowed())
    }
}

/// Borrowed hot-path view; checking existing context receipts adds no strings.
pub(super) struct ScopeRef<'a> {
    pub document: &'a str,
    pub revision: u64,
    pub syntax_context: u64,
    pub expansion: Option<u64>,
    pub start: u64,
    pub end: u64,
}

impl ScopeRef<'_> {
    pub(super) fn covers(&self, current: ScopeRef<'_>) -> bool {
        self.document == current.document && self.revision == current.revision
            && self.syntax_context == current.syntax_context && self.expansion == current.expansion
            && self.start <= current.start && current.start < current.end && current.end <= self.end
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ValueSubject {
    pub source: SourceIdentity,
    pub generation: u64,
    /// Host analysis lifetime; a reopen must not reuse an old observation even
    /// when the client restarts document revision numbering.
    pub analysis_epoch: u64,
    pub owner: String,
    pub producer_registration_id: String,
    pub scope: ScopeSubject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct JavaIdentity {
    pub id: String,
    pub name: String,
    pub loader_id: String,
    /// A JAR or runtime-image digest, not a guessed origin from a class name.
    pub origin_sha256: String,
    pub kind: String,
    pub ancestry_complete: bool,
    /// Complete transitive ancestry, expressed as identities within this capture.
    pub ancestors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RegistrationBinding {
    pub registration_id: String,
    pub class_id: String,
    pub class_info_class_id: String,
    pub class_info_instance: String,
    pub exact_class_info_instance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CaptureRegistry {
    pub version: u32,
    pub source: SourceIdentity,
    pub generation_start: u64,
    pub generation_end: u64,
    pub complete: bool,
    pub entries: Vec<RegistrationBinding>,
    pub classes: Vec<JavaIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OriginClaim {
    pub source_model: String,
    pub artifact_sha256: String,
    pub route: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProducerClaim {
    pub version: u32,
    pub subject: ValueSubject,
    pub public_bound: String,
    pub implementation_upper_bound: String,
    pub complete: bool,
    pub single: bool,
    pub maybe_unset: bool,
    pub before_change: String,
    pub origin: Option<OriginClaim>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Version, Source, Generation, Incomplete, Limit, Identity, Routing,
    Subject, Origin, Domain, Storage,
}

/// Conditional mathematical result only. Authentication/publication is absent.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ConditionalStorage {
    pub public_bound: String,
    pub implementation_upper_bound: String,
    pub maybe_unset: bool,
}

pub(super) struct CheckedCaptureRegistry<'a> {
    source: SourceIdentity,
    generation: u64,
    records: &'a [Value],
    graph: Graph,
}

fn bounded(value: &str) -> bool { !value.is_empty() && value.len() <= 1024 }
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl CaptureRegistry {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, Failure> {
        if bytes.len() > 1024 * 1024 { return Err(Failure::Limit); }
        serde_json::from_slice(bytes).map_err(|_| Failure::Version)
    }

    /// Checks correspondence to the retained catalog, not truth of the capture.
    /// The caller must obtain expected_generation independently, not from self.
    pub(super) fn check<'a>(&self, current: &SourceIdentity, expected_generation: u64, records: &'a [Value])
        -> Result<CheckedCaptureRegistry<'a>, Failure>
    {
        if self.version != 1 { return Err(Failure::Version); }
        if !digest(&current.snapshot_id) || !digest(&current.source_digest) || &self.source != current {
            return Err(Failure::Source);
        }
        if expected_generation == 0 || self.generation_start != expected_generation
            || self.generation_end != expected_generation { return Err(Failure::Generation); }
        if !self.complete || self.entries.len() != records.len() { return Err(Failure::Incomplete); }
        if records.is_empty() || records.len() > 1024 || self.classes.is_empty() || self.classes.len() > 8192 {
            return Err(Failure::Limit);
        }
        let mut identities = BTreeMap::new();
        let mut names = BTreeSet::new();
        for class in &self.classes {
            if !bounded(&class.id) || !bounded(&class.name) || !bounded(&class.loader_id)
                || !digest(&class.origin_sha256) || !class.ancestry_complete || class.ancestors.len() > 1024
                || !names.insert(class.name.as_str()) || identities.insert(class.id.as_str(), class).is_some() {
                // Name-only Catalog APIs cannot safely represent same-name loaders.
                return Err(Failure::Identity);
            }
        }
        let mut classes = Vec::new();
        for class in &self.classes {
            let ancestors = class.ancestors.iter().map(|id| identities.get(id.as_str()).copied())
                .collect::<Option<Vec<_>>>().ok_or(Failure::Identity)?;
            let ids = class.ancestors.iter().map(String::as_str).collect::<BTreeSet<_>>();
            if ids.len() != ancestors.len() || ids.contains(class.id.as_str())
                || ancestors.iter().any(|a| a.ancestors.iter().any(|id| !ids.contains(id.as_str()))) {
                return Err(Failure::Identity);
            }
            let concrete = ancestors.iter().filter(|a| !matches!(a.kind.as_str(), "Interface" | "Annotation")).collect::<Vec<_>>();
            if concrete.iter().any(|a| concrete.iter().any(|b| a.id != b.id
                && !a.ancestors.contains(&b.id) && !b.ancestors.contains(&a.id))) {
                return Err(Failure::Identity);
            }
            classes.push(Class { name:class.name.clone(), kind:class.kind.clone(), super_class:None,
                interfaces:ancestors.iter().map(|a|a.name.clone()).collect(), lineage_complete:true });
        }
        let graph = Graph::new(classes).ok_or(Failure::Identity)?;
        let mut registrations = BTreeSet::new();
        let mut instances = BTreeSet::new();
        for (entry, record) in self.entries.iter().zip(records) {
            let class = identities.get(entry.class_id.as_str()).ok_or(Failure::Identity)?;
            let info = identities.get(entry.class_info_class_id.as_str()).ok_or(Failure::Identity)?;
            let runtime = &record["typeRuntime"];
            let ancestors = runtime["ancestors"].as_array().ok_or(Failure::Identity)?;
            if ancestors.len() > 1024 { return Err(Failure::Limit); }
            let raw_ancestors = ancestors.iter().map(Value::as_str).collect::<Option<BTreeSet<_>>>()
                .ok_or(Failure::Identity)?;
            let identity_ancestors = class.ancestors.iter().map(|id| identities[id.as_str()].name.as_str())
                .collect::<BTreeSet<_>>();
            if !bounded(&entry.registration_id) || !registrations.insert(entry.registration_id.as_str())
                || !bounded(&entry.class_info_instance) || !instances.insert(entry.class_info_instance.as_str())
                || entry.exact_class_info_instance != entry.class_info_instance
                || record["registrationId"].as_str() != Some(entry.registration_id.as_str())
                || record["originalClass"].as_str() != Some(class.name.as_str())
                || runtime["version"] != 3 || runtime["classInfoClass"].as_str() != Some(info.name.as_str())
                || runtime["classKind"].as_str() != Some(class.kind.as_str()) || runtime["ancestryComplete"] != true
                || raw_ancestors.len() != ancestors.len() || raw_ancestors != identity_ancestors {
                return Err(Failure::Routing);
            }
        }
        RegistryGeometry::check(records, &graph).ok_or(Failure::Routing)?;
        Ok(CheckedCaptureRegistry { source:current.clone(), generation:expected_generation, records, graph })
    }
}

impl CheckedCaptureRegistry<'_> {
    /// OriginClaim remains a claim. A successful conditional result does not
    /// attest native-event origin, collector correctness or source-model truth.
    pub(super) fn evaluate(&self, claim: &ProducerClaim, current: &ValueSubject, slot: &str)
        -> Result<ConditionalStorage, Failure>
    {
        if claim.version != 1 { return Err(Failure::Version); }
        if claim.subject != *current || current.source != self.source || current.generation != self.generation
            || current.analysis_epoch == 0
            || !bounded(&current.owner) || !bounded(&current.producer_registration_id)
            || !bounded(&current.scope.document) || !current.scope.covers(&current.scope) {
            return Err(Failure::Subject);
        }
        let origin = claim.origin.as_ref().ok_or(Failure::Origin)?;
        if !bounded(&origin.source_model) || !digest(&origin.artifact_sha256) || !bounded(&origin.route) {
            return Err(Failure::Origin);
        }
        if !claim.complete || !claim.single || claim.before_change != "default-clone"
            || !bounded(&claim.public_bound) || !bounded(&claim.implementation_upper_bound) {
            return Err(Failure::Domain);
        }
        if !RegistryGeometry::check(self.records, &self.graph).is_some_and(|registry|
            registry.preserves(&claim.public_bound, &claim.implementation_upper_bound, slot)) {
            return Err(Failure::Storage);
        }
        Ok(ConditionalStorage { public_bound:claim.public_bound.clone(),
            implementation_upper_bound:claim.implementation_upper_bound.clone(), maybe_unset:claim.maybe_unset })
    }
}

#[cfg(test)]
mod tests;
