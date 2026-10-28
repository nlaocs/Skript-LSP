//! Component-bound immutable JSON datasets, separate from the SSG Catalog.
use std::{collections::{BTreeMap, BTreeSet}, io::Read, path::Path, sync::Arc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHUNK_BYTES: u32 = 64 * 1024;

/// Explicit client authorization of a dataset and the component binary that reads it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddonDataPin {
    pub component_id: String,
    pub component_sha256: String,
    pub id: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub sha256: String,
    pub bindings: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    format_version: u32,
    component_id: String,
    id: String,
    schema_id: String,
    schema_version: u32,
    bindings: BTreeMap<String, String>,
    payload: Box<serde_json::value::RawValue>,
}

/// Verified bytes captured once. Clones, including recreated guest Stores, share them.
#[derive(Clone, Debug)]
pub struct AddonDataSource {
    pin: AddonDataPin,
    bytes: Arc<[u8]>,
    envelope_bytes: usize,
    content_sha256: String,
}

impl AddonDataSource {
    /// Original envelope size charged against the shared retained-input budget.
    pub fn byte_length(&self) -> usize { self.envelope_bytes }

    pub fn from_json(pin: AddonDataPin, bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_SOURCE_BYTES { return Err("addon dataset exceeds 4 MiB".into()); }
        if !valid_id(&pin.component_id) || !valid_id(&pin.id) || !valid_id(&pin.schema_id)
            || pin.schema_version == 0 || !valid_sha(&pin.sha256) || !valid_sha(&pin.component_sha256)
            || pin.bindings.len() > 32 || pin.bindings.iter().any(|(k,v)| !valid_id(k)
                || v.is_empty() || v.len() > 512 || v.chars().any(char::is_control)) {
            return Err("invalid addon dataset pin".into());
        }
        if digest(bytes) != pin.sha256 { return Err("addon dataset SHA-256 mismatch".into()); }
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| format!("invalid addon dataset JSON: {e}"))?;
        if envelope.format_version != 1 || envelope.component_id != pin.component_id
            || envelope.id != pin.id || envelope.schema_id != pin.schema_id
            || envelope.schema_version != pin.schema_version || envelope.bindings != pin.bindings {
            return Err("addon dataset schema/owner/identity mismatch".into());
        }
        let content_sha256=digest(envelope.payload.get().as_bytes());
        Ok(Self { bytes: Arc::from(envelope.payload.get().as_bytes()), pin, envelope_bytes: bytes.len(),content_sha256 })
    }

    /// Bounded even if the file grows between metadata and read. Never re-read by guests.
    pub fn from_file(pin: AddonDataPin, path: impl AsRef<Path>) -> Result<Self, String> {
        Self::from_file_with_budget(pin,path,MAX_SOURCE_BYTES)
    }

    /// Apply a caller's remaining aggregate budget before reading/retaining bytes.
    pub fn from_file_with_budget(pin: AddonDataPin, path: impl AsRef<Path>, remaining: usize) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("cannot open addon dataset: {e}"))?;
        let limit=remaining.min(MAX_SOURCE_BYTES);
        if file.metadata().map_err(|e| e.to_string())?.len() > limit as u64 {
            return Err("addon dataset exceeds source/remaining aggregate byte budget".into());
        }
        let mut bytes = Vec::new();
        file.take((limit + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len()>limit {return Err("addon dataset grew beyond remaining byte budget".into());}
        Self::from_json(pin, &bytes)
    }

    fn authorized(&self, component: &(String, String)) -> bool {
        self.pin.component_id == component.0 && self.pin.component_sha256 == component.1
    }
}

pub(super) fn validate_sources(sources: &[AddonDataSource]) -> bool {
    let mut identities = BTreeSet::new();
    let mut bytes = 0usize;
    sources.len() <= 64 && sources.iter().all(|source| {
        bytes = bytes.saturating_add(source.envelope_bytes);
        bytes <= MAX_TOTAL_BYTES && identities.insert((&source.pin.component_id, &source.pin.id))
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
fn valid_sha(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

use crate::bindings::nlaocs::skript_parser_addon::addon_data as wit;
impl wit::Host for super::StoreData {
    fn info(&mut self, id: String) -> Result<Option<wit::SourceInfo>, wit::Error> {
        self.source_query(None).map_err(|e|wit::Error {kind:wit::ErrorKind::InvalidInput,message:e.message})?;
        let owner = self.addon_data_owner.as_ref().ok_or_else(denied)?;
        let info=self.addon_data_sources.iter().find(|s| s.pin.id == id && s.authorized(owner)).map(|source| wit::SourceInfo {
            id: source.pin.id.clone(), schema_id: source.pin.schema_id.clone(), schema_version: source.pin.schema_version,
            envelope_sha256:source.pin.sha256.clone(),content_sha256: source.content_sha256.clone(), bindings: source.pin.bindings.iter()
                .map(|(key,value)| wit::Binding { key:key.clone(), value:value.clone() }).collect(),
            byte_length: source.bytes.len() as u64,
        });
        let bytes=info.as_ref().map_or(0,|s|s.id.len()+s.schema_id.len()+s.envelope_sha256.len()+s.content_sha256.len()+12+s.bindings.iter().map(|b|b.key.len()+b.value.len()).sum::<usize>());
        self.source_output(bytes).map_err(|e|wit::Error {kind:wit::ErrorKind::InvalidInput,message:e.message})?;
        Ok(info)
    }
    fn read(&mut self, id: String, offset: u64, max_bytes: u32) -> Result<Option<wit::Chunk>, wit::Error> {
        self.source_query(None).map_err(|e|wit::Error {kind:wit::ErrorKind::InvalidInput,message:e.message})?;
        let owner = self.addon_data_owner.as_ref().ok_or_else(denied)?;
        if max_bytes == 0 || max_bytes > MAX_CHUNK_BYTES { return Err(invalid()); }
        let Some(source) = self.addon_data_sources.iter().find(|s| s.pin.id == id && s.authorized(owner)) else { return Ok(None); };
        let start = usize::try_from(offset).map_err(|_| invalid())?;
        if start > source.bytes.len() { return Err(invalid()); }
        let end = start.saturating_add(max_bytes as usize).min(source.bytes.len());
        let total_length=source.bytes.len() as u64;
        let bytes=source.bytes[start..end].to_vec();
        let key=format!("addon-data:{}:{}",id,source.content_sha256);
        self.source_query(Some((&key,total_length as usize))).map_err(|e|wit::Error {kind:wit::ErrorKind::InvalidInput,message:e.message})?;
        self.source_output(bytes.len()).map_err(|e|wit::Error {kind:wit::ErrorKind::InvalidInput,message:e.message})?;
        Ok(Some(wit::Chunk {offset,total_length,bytes}))
    }
}
fn denied() -> wit::Error { wit::Error { kind: wit::ErrorKind::AccessDenied, message: "addon data requires a validated component identity".into() } }
fn invalid() -> wit::Error { wit::Error { kind: wit::ErrorKind::InvalidInput, message: "invalid addon data chunk range".into() } }

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(id: &str, payload: serde_json::Value) -> (AddonDataPin, Vec<u8>) {
        let bindings = BTreeMap::from([("loader".into(), "owned-fixture".into())]);
        let bytes = serde_json::to_vec(&serde_json::json!({"formatVersion":1,"componentId":"test.data",
            "id":id,"schemaId":"test.schema","schemaVersion":2,"bindings":bindings,"payload":payload})).unwrap();
        (AddonDataPin {component_id:"test.data".into(),component_sha256:"a".repeat(64),id:id.into(),schema_id:"test.schema".into(),
            schema_version:2,sha256:digest(&bytes),bindings},bytes)
    }
    #[test] fn validates_identity_and_keeps_exact_payload_bytes() {
        let (pin,bytes)=sample("one",serde_json::json!({"text":"日本語"}));
        let source=AddonDataSource::from_json(pin.clone(),&bytes).unwrap();
        assert!(source.authorized(&(pin.component_id.clone(),pin.component_sha256.clone())));
        assert!(!source.authorized(&("other".into(),pin.component_sha256.clone())));
        assert!(!source.authorized(&(pin.component_id.clone(),"b".repeat(64))));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&source.bytes).unwrap()["text"],"日本語");
        assert!(Arc::ptr_eq(&source.bytes,&source.clone().bytes));
    }
    #[test] fn rejects_pin_schema_owner_and_binding_mismatches() {
        let (pin,bytes)=sample("one",serde_json::json!({}));
        let mut bad=pin.clone();bad.schema_version=3;assert!(AddonDataSource::from_json(bad,&bytes).is_err());
        let mut bad=pin.clone();bad.schema_id="other.schema".into();assert!(AddonDataSource::from_json(bad,&bytes).is_err());
        let mut bad=pin.clone();bad.component_id="other".into();assert!(AddonDataSource::from_json(bad,&bytes).is_err());
        let mut bad=pin.clone();bad.bindings.insert("loader".into(),"different".into());assert!(AddonDataSource::from_json(bad,&bytes).is_err());
        let mut bad=pin;bad.sha256="0".repeat(64);assert!(AddonDataSource::from_json(bad,&bytes).is_err());
    }
    #[test] fn rejects_corruption_and_oversize_before_deserializing() {
        let (mut pin,_)=sample("one",serde_json::json!({}));
        let bytes=b"not json";pin.sha256=digest(bytes);assert!(AddonDataSource::from_json(pin.clone(),bytes).unwrap_err().contains("JSON"));
        assert!(AddonDataSource::from_json(pin,&vec![0;MAX_SOURCE_BYTES+1]).unwrap_err().contains("4 MiB"));
    }
    #[test] fn duplicates_and_aggregate_size_are_bounded() {
        let (pin,bytes)=sample("one",serde_json::json!({}));let source=AddonDataSource::from_json(pin,&bytes).unwrap();
        assert!(!validate_sources(&[source.clone(),source]));
        let (pin,bytes)=sample("one",serde_json::json!({"text":"x".repeat(3*1024*1024)}));
        let source=AddonDataSource::from_json(pin,&bytes).unwrap();
        let mut sources=Vec::new();for i in 0..6 {let mut copy=source.clone();copy.pin.id=format!("id-{i}");sources.push(copy);}
        assert!(!validate_sources(&sources));
    }
    #[test] fn captured_source_does_not_follow_file_changes() {
        let (pin,bytes)=sample("one",serde_json::json!({"value":1}));let file=tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(),&bytes).unwrap();let source=AddonDataSource::from_file(pin,file.path()).unwrap();
        std::fs::write(file.path(),b"changed").unwrap();assert_eq!(serde_json::from_slice::<serde_json::Value>(&source.bytes).unwrap()["value"],1);
    }
    #[test] fn remaining_aggregate_budget_is_checked_before_capture() {
        let (pin,bytes)=sample("one",serde_json::json!({"value":1}));let file=tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(),&bytes).unwrap();
        assert!(AddonDataSource::from_file_with_budget(pin.clone(),file.path(),bytes.len()-1).unwrap_err().contains("budget"));
        let source=AddonDataSource::from_file_with_budget(pin,file.path(),bytes.len()).unwrap();
        assert_eq!(source.byte_length(),bytes.len());
        assert_eq!(source.content_sha256,digest(&source.bytes));
    }
    #[test] fn only_validated_owner_binary_reads_bounded_chunks() {
        use wit::Host;
        let (pin,bytes)=sample("one",serde_json::json!({"text":"日本語"}));
        let mut config=super::super::HostConfig::default();
        config.addon_data_sources.push(AddonDataSource::from_json(pin,&bytes).unwrap());
        let engine=wasmtime::Engine::default();
        let mut store=super::super::create_store(&engine,&config,Arc::from([]));
        assert!(matches!(store.data_mut().info("one".into()).unwrap_err().kind,wit::ErrorKind::AccessDenied));
        store.data_mut().addon_data_owner=Some(("other".into(),"a".repeat(64)));
        assert!(store.data_mut().info("one".into()).unwrap().is_none());
        store.data_mut().addon_data_owner=Some(("test.data".into(),"b".repeat(64)));
        assert!(store.data_mut().info("one".into()).unwrap().is_none());
        store.data_mut().addon_data_owner=Some(("test.data".into(),"a".repeat(64)));
        let info=store.data_mut().info("one".into()).unwrap().unwrap();
        assert_eq!(info.envelope_sha256,digest(&bytes));
        let length=info.byte_length;
        let mut payload=Vec::new();
        for offset in 0..length { payload.extend(store.data_mut().read("one".into(),offset,1).unwrap().unwrap().bytes); }
        assert_eq!(info.content_sha256,digest(&payload));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["text"],"日本語");
        assert!(store.data_mut().read("one".into(),length,1).unwrap().unwrap().bytes.is_empty());
        assert!(store.data_mut().read("one".into(),u64::MAX,1).is_err());
        assert!(store.data_mut().read("one".into(),0,0).is_err());
        assert!(store.data_mut().read("one".into(),0,MAX_CHUNK_BYTES+1).is_err());
        let fresh=super::super::create_store(&engine,&config,Arc::from([]));
        assert!(Arc::ptr_eq(&store.data().addon_data_sources[0].bytes,&fresh.data().addon_data_sources[0].bytes));
        assert!(fresh.data().addon_data_owner.is_none());
    }
}
