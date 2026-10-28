//! SSG observation v1 is deliberately NOT CaptureRegistry v1: equal boundary
//! observations do not establish a mutation epoch or authorize storage facts.
use super::{Failure, SourceIdentity, bounded, digest};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct Observation {
    version:u32, capture_id:String, identity_scope:String, generation_evidence:String, getter_scope:String,
    before:Boundary, after:Boundary, consistent:bool, complete:bool, entries:Vec<Entry>,
    exact_bindings:Vec<Exact>, classes:Vec<JavaClass>, loaders:Vec<Loader>, getters:Vec<Getter>, failures:Vec<String>,
}
/// Only the metadata actually consumed by the storage source model. Unknown
/// fields remain available through the immutable host Catalog; skipping them
/// here neither removes their raw-source fence nor supplies semantic authority.
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct ManifestEnvelope {
    schema_version:u32,
    snapshot_id:String,
    #[serde(default)] server:Value,
    #[serde(default)] plugins:Value,
    #[serde(default)] registry_capture:ObservationField,
}

#[derive(Default)]
enum ObservationField { #[default] Absent, Present(Observation) }
impl<'de> Deserialize<'de> for ObservationField {
    fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error> {
        // Only a missing field is Absent. Explicit null/malformed observations
        // must not become a missing optional proof or authorize the legacy path.
        Observation::deserialize(deserializer).map(Self::Present)
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Boundary { complete:bool, registrations_closed:Option<bool>, fingerprint:Option<String> }
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Entry { registration_id:String, order:usize, class_info_instance:String, class_id:Option<String>, class_info_class_id:String, runtime_class_ids:BTreeMap<String,String> }
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Exact { class_id:String, class_info_instance:String }
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct JavaClass { id:String,name:String,catalog_name:String,kind:String,loader_id:Option<String>,module_name:Option<String>,
    module_version:Option<String>,code_origin_sha256:Option<String>,origin_state:String,hidden:bool,ancestry_complete:bool,ancestors:Vec<String> }
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Loader { id:String,class_id:Option<String>,parent_id:Option<String>,parent_known:bool }
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Getter { registration_id:String,resolution_order:usize,event_class_id:String,value_class_id:String,instance_id:Option<String>,class_id:Option<String>,
    state:String,methods:Vec<Method>,unknown_reason:Option<String> }
#[derive(Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Method { declaring_class_id:String,name:String,parameter_class_ids:Vec<String>,return_class_id:String,bridge:bool }

#[derive(Debug)]
pub(in crate::storage) struct BoundObservation {
    source:SourceIdentity,
    manifest_fence:ManifestFence,
    observation:Observation,
}

#[derive(Debug)]
enum ManifestFence { StandaloneSha256([u8;32]), HostCatalogSource }

impl BoundObservation {
    /// The caller must read the exact retained Manifest through its source-bound
    /// Catalog API. A supplied SourceIdentity is an integrity key, not authority.
    pub(super) fn parse(before:&SourceIdentity, bytes:&[u8], after:&SourceIdentity) -> Result<Option<Self>,Failure> {
        Self::split_manifest(before,bytes,after).and_then(|(_,observation)|observation)
    }

    /// Parse the retained Manifest once directly into typed observation fields.
    /// Source-profile checks need neither a large intermediate Value tree nor a
    /// deep clone. The raw-byte fence still covers the unmodified input bytes.
    pub(in crate::storage) fn split_manifest(before:&SourceIdentity, bytes:&[u8], after:&SourceIdentity)
        -> Result<(Value,Result<Option<Self>,Failure>),Failure>
    {
        Self::split_with_fence(before,bytes,after,||ManifestFence::StandaloneSha256(Sha256::digest(bytes).into()))
    }

    /// A verified host read is already fenced by the immutable Catalog's full
    /// raw-source digest. Do not hash that complete Manifest again in guest fuel.
    /// Standalone bytes retain their independent SHA path above.
    pub(in crate::storage) fn split_bound_manifest(document:&crate::storage::document::BoundDocument,after:&SourceIdentity)
        -> Result<(Value,Result<Option<Self>,Failure>),Failure>
    {
        if document.name()!="Manifest.json" {return Err(Failure::Source);}
        Self::split_with_fence(document.source(),document.bytes(),after,||ManifestFence::HostCatalogSource)
    }

    fn split_with_fence(before:&SourceIdentity,bytes:&[u8],after:&SourceIdentity,fence:impl FnOnce()->ManifestFence)
        -> Result<(Value,Result<Option<Self>,Failure>),Failure>
    {
        if bytes.len()>2*1024*1024 {return Err(Failure::Limit);}
        if before!=after || !digest(&before.snapshot_id) || !digest(&before.source_digest) {return Err(Failure::Source);}
        let envelope:ManifestEnvelope=serde_json::from_slice(bytes).map_err(|_|Failure::Version)?;
        if envelope.schema_version!=7 || envelope.snapshot_id!=before.snapshot_id {return Err(Failure::Source);}
        let root=serde_json::json!({"schemaVersion":envelope.schema_version,"snapshotId":envelope.snapshot_id,
            "server":envelope.server,"plugins":envelope.plugins});
        let ObservationField::Present(observation)=envelope.registry_capture else {return Ok((root,Ok(None)));};
        let observation=(||{
            observation.check()?;
            Ok(Some(Self {source:before.clone(),manifest_fence:fence(),observation}))
        })();
        Ok((root,observation))
    }

    pub(in crate::storage) fn matches_source(&self,source:&SourceIdentity) -> bool { &self.source==source }

    pub(super) fn matches_current(&self,source:&SourceIdentity,manifest:&[u8]) -> bool {
        if manifest.len()>2*1024*1024 || source!=&self.source {return false;}
        match self.manifest_fence {
            ManifestFence::StandaloneSha256(hash)=><[u8;32]>::from(Sha256::digest(manifest))==hash,
            // Unbound caller bytes cannot use the trusted host token's fence.
            ManifestFence::HostCatalogSource=>false,
        }
    }

    pub(in crate::storage) fn is_complete(&self)->bool {self.observation.complete}

    /// Only correspondence to retained rows. No conversion into expected
    /// generation, producer origin, ResultType, metadata or a seal exists.
    pub(in crate::storage) fn check_projection(&self,types:&[Value],events:&[Value]) -> Result<(),Failure> {
        let o=&self.observation;
        if !o.complete {return Err(Failure::Incomplete);}
        if o.entries.len()!=types.len() || o.getters.len()!=events.len() {return Err(Failure::Incomplete);}
        let classes=o.classes.iter().map(|c|(c.id.as_str(),c)).collect::<BTreeMap<_,_>>();
        let mut registrations=BTreeSet::new();let mut instances=BTreeSet::new();
        for (i,(entry,row)) in o.entries.iter().zip(types).enumerate() {
            let class=entry.class_id.as_ref().and_then(|id|classes.get(id.as_str())).ok_or(Failure::Identity)?;
            let runtime=&row["typeRuntime"];
            let ancestors=runtime["ancestors"].as_array().ok_or(Failure::Identity)?;
            if ancestors.len()>1024 {return Err(Failure::Limit);}
            let actual_ancestors=ancestors.iter().map(Value::as_str).collect::<Option<BTreeSet<_>>>().ok_or(Failure::Identity)?;
            let observed_ancestors=class.ancestors.iter().map(|id|classes[id.as_str()].catalog_name.as_str()).collect::<BTreeSet<_>>();
            if entry.order!=i || !registrations.insert(&entry.registration_id) || !instances.insert(&entry.class_info_instance)
                || row["registrationId"].as_str()!=Some(entry.registration_id.as_str())
                || row["typeParseOrder"].as_u64()!=Some(i as u64) || row["originalClass"].as_str()!=Some(class.catalog_name.as_str())
                || row["typeRuntime"]["classInfoClass"].as_str()!=Some(classes[entry.class_info_class_id.as_str()].catalog_name.as_str())
                || runtime["version"]!=3 || runtime["classKind"].as_str()!=Some(class.kind.as_str())
                || runtime["ancestryComplete"].as_bool()!=Some(class.ancestry_complete)
                || actual_ancestors.len()!=ancestors.len() || actual_ancestors!=observed_ancestors
                || o.exact_bindings.iter().filter(|b|Some(&b.class_id)==entry.class_id.as_ref()&&b.class_info_instance==entry.class_info_instance).count()!=1 {
                return Err(Failure::Routing);
            }
            for role in ["classGetterDeclaringClass","classValue","classInfoClass","cloneMethodDeclaringClass",
                "clonerGetterDeclaringClass","clonerClass","serializeAsGetterDeclaringClass","serializeAsClass"] {
                match (runtime[role].as_str(),entry.runtime_class_ids.get(role)) {
                    (None,None)=>{},(Some(name),Some(id)) if classes[id.as_str()].catalog_name==name=>{},
                    _=>return Err(Failure::Identity),
                }
            }
        }
        if o.exact_bindings.len()!=types.len() {return Err(Failure::Routing);}
        for (getter,row) in o.getters.iter().zip(events) {
            if row["registrationId"].as_str()!=Some(getter.registration_id.as_str())
                || row["resolutionOrder"].as_u64()!=Some(getter.resolution_order as u64)
                || row["eventClass"].as_str()!=Some(classes[getter.event_class_id.as_str()].catalog_name.as_str())
                || row["valueClass"].as_str()!=Some(classes[getter.value_class_id.as_str()].catalog_name.as_str()) {return Err(Failure::Routing);}
        }
        Ok(())
    }

    /// Explicit blockers for promotion into the stronger CaptureRegistry and
    /// ProducerClaim contracts. These observations never provide an independent
    /// mutation epoch or authenticate a native producer, even for an observed
    /// (non-hidden) getter. No storage certificate can be issued from this API.
    pub(in crate::storage) fn storage_gaps(&self,registration:Option<&str>) -> Vec<StorageGap> {
        let mut gaps=vec![StorageGap::GenerationUnproven,StorageGap::ProducerOriginUnproven];
        if let Some(registration)=registration {
            let mut getters=self.observation.getters.iter().filter(|g|g.registration_id==registration);
            match (getters.next(),getters.next()) {
                (None,_)=>gaps.push(StorageGap::GetterAbsent),
                (Some(_),Some(_))=>gaps.push(StorageGap::GetterAmbiguous),
                (Some(g),None) if g.state!="observed"=>gaps.push(StorageGap::GetterImplementationUnproven),
                _=>{}
            }
        }
        gaps
    }
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(in crate::storage) enum StorageGap {
    GenerationUnproven,ProducerOriginUnproven,GetterAbsent,GetterAmbiguous,GetterImplementationUnproven,
}

impl Observation {
    fn check(&self)->Result<(),Failure> {
        if self.version!=1 || self.identity_scope!="capture-local" || self.generation_evidence!="boundary-observed-no-mutation-epoch"
            || self.getter_scope!="retained-event-value-rows" {return Err(Failure::Version);}
        if !bounded(&self.capture_id) || self.entries.len()>1024 || self.exact_bindings.len()>1024
            || self.classes.len()>8192 || self.loaders.len()>513 || self.getters.len()>4096 || self.failures.len()>32
            || self.failures.iter().any(|s|!bounded(s)) {return Err(Failure::Limit);}
        let consistent=self.before.complete&&self.after.complete&&self.before.registrations_closed==Some(true)
            &&self.after.registrations_closed==Some(true)&&self.before.fingerprint.is_some()&&self.before.fingerprint==self.after.fingerprint;
        if self.consistent!=consistent || self.complete&&(!consistent||!self.failures.is_empty())
            || [&self.before,&self.after].iter().any(|b|b.fingerprint.as_ref().is_some_and(|s|!digest(s))) {return Err(Failure::Generation);}
        let classes=self.classes.iter().map(|c|(c.id.as_str(),c)).collect::<BTreeMap<_,_>>();
        let loaders=self.loaders.iter().map(|l|(l.id.as_str(),l)).collect::<BTreeMap<_,_>>();
        if classes.len()!=self.classes.len() || loaders.len()!=self.loaders.len() {return Err(Failure::Identity);}
        for class in &self.classes {
            if !bounded(&class.id)||!bounded(&class.name)||!bounded(&class.catalog_name)||!bounded(&class.kind)
                || class.ancestors.len()>1024 || class.ancestors.iter().collect::<BTreeSet<_>>().len()!=class.ancestors.len()
                || class.ancestors.iter().any(|id|id==&class.id||!classes.contains_key(id.as_str()))
                || class.loader_id.as_ref().is_some_and(|id|!loaders.contains_key(id.as_str()))
                || class.module_name.as_ref().is_some_and(|s|!bounded(s)) || class.module_version.as_ref().is_some_and(|s|!bounded(s))
                || class.code_origin_sha256.as_ref().is_some_and(|s|!digest(s))
                || !matches!(class.origin_state.as_str(),"code-source-file"|"runtime-module-no-image-digest"|"unavailable")
                || (class.origin_state=="code-source-file")!=class.code_origin_sha256.is_some() {return Err(Failure::Identity);}
            // hidden and ancestryComplete are observations, not source authority.
            let _=(class.hidden,class.ancestry_complete);
        }
        for loader in &self.loaders {
            if !bounded(&loader.id)||loader.class_id.as_ref().is_some_and(|id|!classes.contains_key(id.as_str()))
                || loader.parent_id.as_ref().is_some_and(|id|id==&loader.id||!loaders.contains_key(id.as_str())) {return Err(Failure::Identity);}
            let _=loader.parent_known;
            let mut seen=BTreeSet::new();let mut next=Some(loader.id.as_str());
            while let Some(id)=next {if !seen.insert(id){return Err(Failure::Identity);}next=loaders[id].parent_id.as_deref();}
        }
        for entry in &self.entries {
            if !bounded(&entry.registration_id)||!bounded(&entry.class_info_instance)
                || !classes.contains_key(entry.class_info_class_id.as_str())
                || entry.class_id.as_ref().is_some_and(|id|!classes.contains_key(id.as_str())) {return Err(Failure::Identity);}
            if entry.runtime_class_ids.len()>8 || entry.runtime_class_ids.iter().any(|(role,id)|
                !matches!(role.as_str(),"classGetterDeclaringClass"|"classValue"|"classInfoClass"|"cloneMethodDeclaringClass"|
                    "clonerGetterDeclaringClass"|"clonerClass"|"serializeAsGetterDeclaringClass"|"serializeAsClass")||!classes.contains_key(id.as_str())) {return Err(Failure::Identity);}
        }
        for binding in &self.exact_bindings {
            if !classes.contains_key(binding.class_id.as_str())||!bounded(&binding.class_info_instance) {return Err(Failure::Identity);}
        }
        for getter in &self.getters {
            if !bounded(&getter.registration_id)||getter.methods.len()>16
                || !classes.contains_key(getter.event_class_id.as_str())||!classes.contains_key(getter.value_class_id.as_str())
                || getter.class_id.as_ref().is_some_and(|id|!classes.contains_key(id.as_str()))
                || getter.instance_id.as_ref().is_some_and(|s|!bounded(s))
                || match getter.state.as_str() {"observed"=>getter.class_id.is_none()||getter.instance_id.is_none()||getter.methods.is_empty()||getter.unknown_reason.is_some(),
                    "unknown"=>getter.unknown_reason.as_ref().is_none_or(|s|!bounded(s)),_=>true} {return Err(Failure::Identity);}
            for method in &getter.methods {
                if !bounded(&method.name)||method.parameter_class_ids.len()>255||!classes.contains_key(method.declaring_class_id.as_str())
                    ||!classes.contains_key(method.return_class_id.as_str())||method.parameter_class_ids.iter().any(|id|!classes.contains_key(id.as_str())) {return Err(Failure::Identity);}
                let _=method.bridge;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
