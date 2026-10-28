//! Eligibility only. A decoded header is not a checked registry projection.
use super::{Failure,SourceIdentity,bounded,digest};
use super::observation::StorageGap;
use crate::storage::document::BoundDocument;
use serde::{Deserialize,Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct Envelope {
    schema_version:u32,snapshot_id:String,
    #[serde(default)] server:Value,
    #[serde(default)] plugins:Value,
    #[serde(default)] registry_capture:Field,
}
#[derive(Default)]
enum Field { #[default] Absent, Present(Header) }
impl<'de> Deserialize<'de> for Field {
    fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error> {Header::deserialize(d).map(Self::Present)}
}
#[derive(Deserialize,Serialize)]
#[serde(rename_all="camelCase")]
struct Header {
    version:u32,capture_id:String,identity_scope:String,generation_evidence:String,getter_scope:String,
    before:Boundary,after:Boundary,consistent:bool,complete:bool,failures:Vec<String>,
    #[serde(default,skip_serializing_if="Option::is_none")] interval:Option<super::observation::interval::Interval>,
    // Detail fields are intentionally not deserialized or called valid here.
}
#[derive(Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Boundary {complete:bool,registrations_closed:Option<bool>,fingerprint:Option<String>}

pub(in crate::storage) struct DeferredObservation {source:SourceIdentity,declared_complete:bool,header:Header}
impl DeferredObservation {
    pub(in crate::storage) fn source_identity(&self)->&SourceIdentity {&self.source}
    pub(in crate::storage) fn version(&self)->u32 {self.header.version}
    pub(in crate::storage) fn header_value(&self)->Result<Value,Failure> {serde_json::to_value(&self.header).map_err(|_|Failure::Version)}

    pub(in crate::storage) fn matches_source(&self,source:&SourceIdentity)->bool {&self.source==source}
    pub(in crate::storage) fn declared_complete(&self)->bool {self.declared_complete}
    pub(in crate::storage) fn gaps(&self)->[StorageGap;2] {
        [StorageGap::GenerationUnproven,StorageGap::ProducerOriginUnproven]
    }
}

/// v1 declares boundary observations, not the independent generation/native
/// producer inputs required by CaptureRegistry/ProducerClaim. Reject eligibility
/// before detail work, never after a trap. No certificate can leave this API.
pub(in crate::storage) fn prepare(document:&BoundDocument,after:&SourceIdentity)
    -> Result<(Value,Result<Option<DeferredObservation>,Failure>),Failure>
{
    let before=document.source();
    if document.name()!="Manifest.json" || before!=after || !digest(&before.snapshot_id)||!digest(&before.source_digest) {return Err(Failure::Source);}
    let envelope:Envelope=serde_json::from_slice(document.bytes()).map_err(|_|Failure::Version)?;
    if envelope.schema_version!=7 || envelope.snapshot_id!=before.snapshot_id {return Err(Failure::Source);}
    let metadata=serde_json::json!({"schemaVersion":envelope.schema_version,"snapshotId":envelope.snapshot_id,"server":envelope.server,"plugins":envelope.plugins});
    let Field::Present(h)=envelope.registry_capture else {return Ok((metadata,Ok(None)));};
    let result=(||{
        if !matches!(h.version,1|2)||h.identity_scope!="capture-local"
            ||h.generation_evidence!=if h.version==1 {"boundary-observed-no-mutation-epoch"}else{"supported-api-interval-with-quiescent-premise-v1"}
            ||h.getter_scope!="retained-event-value-rows"||h.version==1&&h.interval.is_some(){return Err(Failure::Version);}
        if !bounded(&h.capture_id)||h.failures.len()>32||h.failures.iter().any(|s|!bounded(s)) {return Err(Failure::Limit);}
        let consistent=h.before.complete&&h.after.complete&&h.before.registrations_closed==Some(true)&&h.after.registrations_closed==Some(true)
            && h.before.fingerprint.is_some()&&h.before.fingerprint==h.after.fingerprint;
        if h.consistent!=consistent||h.complete&&(!consistent||!h.failures.is_empty())
            || [&h.before,&h.after].iter().any(|b|b.fingerprint.as_ref().is_some_and(|s|!digest(s))) {return Err(Failure::Generation);}
        Ok(Some(DeferredObservation {source:before.clone(),declared_complete:h.complete,header:h}))
    })();
    Ok((metadata,result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::document::{self,Chunk};
    #[test]
    fn header_eligibility_never_claims_ignored_details_are_checked() {
        let raw=include_bytes!("observation/data/real-capture/reflect-manifest.json");
        let mut value:Value=serde_json::from_slice(raw).unwrap();
        let source=SourceIdentity {snapshot_id:value["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
        // A bad detail value remains UNCHECKED, not a valid registry. The v1
        // prerequisite failure cannot grant storage authority in either case.
        for bad in [Value::Null,serde_json::json!("not-an-array")] {
            value["registryCapture"]["classes"]=bad;let bytes=serde_json::to_vec(&value).unwrap();
            let doc=document::mock_document(&source,"Manifest.json",||Ok(source.clone()),|offset,max|{
                let start=offset as usize;Ok(Some(Chunk {offset,total_length:bytes.len() as u64,bytes:bytes[start..bytes.len().min(start+max as usize)].to_vec()}))
            }).unwrap().unwrap();
            let (_,header)=prepare(&doc,&source).unwrap();let deferred=header.unwrap().unwrap();
            assert!(deferred.matches_source(&source));assert_eq!(deferred.gaps(),[StorageGap::GenerationUnproven,StorageGap::ProducerOriginUnproven]);
        }
    }

    #[test]
    fn missing_null_and_conflicting_header_claims_stay_distinct() {
        let original:Value=serde_json::from_slice(include_bytes!("observation/data/real-capture/core-manifest.json")).unwrap();
        let source=SourceIdentity {snapshot_id:original["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
        for mode in ["missing","null","conflict","incomplete"] {
            let mut value=original.clone();
            match mode {
                "missing"=>{value.as_object_mut().unwrap().remove("registryCapture");},
                "null"=>value["registryCapture"]=Value::Null,
                "conflict"=>value["registryCapture"]["consistent"]=Value::Bool(false),
                _=>{value["registryCapture"]["complete"]=Value::Bool(false);value["registryCapture"]["failures"]=serde_json::json!(["incomplete-control"]);}
            }
            let bytes=serde_json::to_vec(&value).unwrap();
            let doc=document::mock_document(&source,"Manifest.json",||Ok(source.clone()),|offset,max|{
                let start=offset as usize;Ok(Some(Chunk {offset,total_length:bytes.len() as u64,bytes:bytes[start..bytes.len().min(start+max as usize)].to_vec()}))
            }).unwrap().unwrap();
            match mode {
                "missing"=>assert!(prepare(&doc,&source).unwrap().1.unwrap().is_none()),
                "null"=>assert_eq!(prepare(&doc,&source).err(),Some(Failure::Version)),
                "conflict"=>assert!(matches!(prepare(&doc,&source).unwrap().1,Err(Failure::Generation))),
                _=>assert!(!prepare(&doc,&source).unwrap().1.unwrap().unwrap().declared_complete()),
            }
        }
    }
}
