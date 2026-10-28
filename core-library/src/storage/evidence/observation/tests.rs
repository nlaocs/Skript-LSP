use super::*;
use serde_json::json;

fn source() -> SourceIdentity {SourceIdentity {snapshot_id:"a".repeat(64),source_digest:"b".repeat(64)}}
fn fixture() -> Value {serde_json::from_str(include_str!("data/ssg-observation-v1.json")).unwrap()}
fn manifest(fixture:&Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"schemaVersion":7,"snapshotId":source().snapshot_id,"registryCapture":fixture["registryCapture"]})).unwrap()
}
fn read(f:&Value) -> Result<BoundObservation,Failure> {
    BoundObservation::parse(&source(),&manifest(f),&source()).map(|o|o.unwrap())
}

#[test]
fn self_authored_ssg_export_roundtrips_without_becoming_a_storage_proof() {
    let f=fixture();let bound=read(&f).unwrap();
    let types=f["types"].as_array().unwrap();let events=f["eventValues"].as_array().unwrap();
    assert_eq!(bound.check_projection(types,events),Ok(()));
    assert!(bound.matches_current(&source(),&manifest(&f)));
    assert_eq!(bound.observation.generation_evidence,"boundary-observed-no-mutation-epoch");
    assert!(serde_json::from_value::<super::super::CaptureRegistry>(f["registryCapture"].clone()).is_err(),
        "boundary observations cannot be read as the stronger generation contract");
}

#[test]
fn raw_manifest_and_source_identity_fence_cached_observations() {
    let f=fixture();let bytes=manifest(&f);let bound=read(&f).unwrap();
    let mut changed=f.clone();changed["registryCapture"]["captureId"]=json!("another-capture");
    assert!(!bound.matches_current(&source(),&manifest(&changed)),"snapshotId does not hash optional Manifest observations");
    let mut newer=source();newer.source_digest="c".repeat(64);
    assert!(!bound.matches_current(&newer,&bytes));
    assert_eq!(BoundObservation::parse(&source(),&bytes,&newer).err(),Some(Failure::Source));
    let old=serde_json::to_vec(&json!({"schemaVersion":7,"snapshotId":source().snapshot_id})).unwrap();
    assert!(BoundObservation::parse(&source(),&old,&source()).unwrap().is_none());
}

#[test]
fn observed_epochs_and_missing_identity_are_not_upgraded_or_repaired() {
    let f=fixture();
    for field in ["version","generationEvidence","identityScope","getterScope","class","loader","getter","cycle","consistent"] {
        let mut changed=f.clone();let o=&mut changed["registryCapture"];
        match field {
            "version"=>o[field]=json!(2),"generationEvidence"=>o[field]=json!("complete-mutation-epoch"),
            "identityScope"=>o[field]=json!("global"),"getterScope"=>o[field]=json!("all-runtime-event-values"),
            "class"=>o["entries"][0]["classId"]=json!("missing"),
            "loader"=>o["classes"][0]["loaderId"]=json!("missing"),
            "getter"=>o["getters"][0]["methods"][0]["declaringClassId"]=json!("missing"),
            "cycle"=>{let id=o["loaders"][0]["id"].clone();o["loaders"][0]["parentId"]=id;},
            _=>o[field]=json!(false),
        }
        assert!(read(&changed).is_err(),"{field}");
    }
    assert_eq!(BoundObservation::parse(&source(),&vec![b' ';2*1024*1024+1],&source()).err(),Some(Failure::Limit));
}

#[test]
fn source_rows_exact_bindings_and_unknown_getters_stay_distinct() {
    let f=fixture();let bound=read(&f).unwrap();
    let types=f["types"].as_array().unwrap().clone();let events=f["eventValues"].as_array().unwrap().clone();
    let mut changed=types.clone();changed[0]["registrationId"]=json!("other");
    assert_eq!(bound.check_projection(&changed,&events),Err(Failure::Routing));
    changed=types.clone();changed[0]["typeRuntime"]["classInfoClass"]=json!("other");
    assert_eq!(bound.check_projection(&changed,&events),Err(Failure::Routing));
    let mut changed_events=events.clone();changed_events[0]["registrationId"]=json!("other");
    assert_eq!(bound.check_projection(&types,&changed_events),Err(Failure::Routing));
    let mut unknown=f.clone();let getter=&mut unknown["registryCapture"]["getters"][0];
    getter["state"]=json!("unknown");getter["methods"]=json!([]);getter["unknownReason"]=json!("hidden-callable-implementation");
    assert_eq!(read(&unknown).unwrap().check_projection(&types,&events),Ok(()),"unknown getter observation is not certified as callable semantics");
    let mut partial=f.clone();partial["registryCapture"]["complete"]=json!(false);partial["registryCapture"]["failures"]=json!(["capture-output-size-limit"]);
    assert_eq!(read(&partial).unwrap().check_projection(&types,&events),Err(Failure::Incomplete));
    let mut extra=f.clone();let binding=extra["registryCapture"]["exactBindings"][0].clone();extra["registryCapture"]["exactBindings"].as_array_mut().unwrap().push(binding);
    assert_eq!(read(&extra).unwrap().check_projection(&types,&events),Err(Failure::Routing));
}


#[test]
fn compact_ssg_manifest_keeps_projection_and_budget_is_not_authority() {
    let f:Value=serde_json::from_str(include_str!("data/ssg-compact-fixture-v1.json")).unwrap();
    let bytes=f["rawManifest"].as_str().unwrap().as_bytes();
    let types=f["types"].as_array().unwrap();let events=f["eventValues"].as_array().unwrap();
    let bound=BoundObservation::parse(&source(),bytes,&source()).unwrap().unwrap();
    assert_eq!(bound.check_projection(types,events),Ok(()));
    assert!(bound.matches_current(&source(),bytes));
    let mut changed:Value=serde_json::from_slice(bytes).unwrap();
    changed["registryCaptureBudget"]["originalCounts"]["entries"]=json!(999);
    changed["registryCaptureBudget"]["truncated"]=json!(false);
    let changed_bytes=serde_json::to_vec(&changed).unwrap();
    assert!(!bound.matches_current(&source(),&changed_bytes),"budget changes still invalidate the exact raw-Manifest fence");
    let diagnostic_only=BoundObservation::parse(&source(),&changed_bytes,&source()).unwrap().unwrap();
    assert_eq!(diagnostic_only.check_projection(types,events),Ok(()),"diagnostic dimensions are not registry facts");
    changed["registryCapture"]["complete"]=json!(false);
    changed["registryCapture"]["failures"]=json!(["capture-output-size-limit"]);
    changed["registryCaptureBudget"]["claimedComplete"]=json!(true);
    let incomplete=BoundObservation::parse(&source(),&serde_json::to_vec(&changed).unwrap(),&source()).unwrap().unwrap();
    assert_eq!(incomplete.check_projection(types,events),Err(Failure::Incomplete),"budget metadata cannot repair or certify discarded evidence");
}

#[test]
fn actual_complete_captures_project_in_rust_but_do_not_certify_hidden_or_observed_producers() {
    for (m,t,e) in [
        (include_bytes!("data/real-capture/core-manifest.json").as_slice(),include_str!("data/real-capture/core-types.json"),include_str!("data/real-capture/core-event-values.json")),
        (include_bytes!("data/real-capture/reflect-manifest.json").as_slice(),include_str!("data/real-capture/reflect-types.json"),include_str!("data/real-capture/reflect-event-values.json")),
    ] {
        let manifest:Value=serde_json::from_slice(m).unwrap();
        let source=SourceIdentity {snapshot_id:manifest["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
        let bound=BoundObservation::parse(&source,m,&source).unwrap().unwrap();
        let types:Vec<Value>=serde_json::from_str(t).unwrap();let events:Vec<Value>=serde_json::from_str(e).unwrap();
        assert_eq!(bound.check_projection(&types,&events),Ok(()));assert!(bound.matches_current(&source,m));
        let player=bound.observation.getters.iter().find(|g|g.registration_id=="event-value:skript:4c5774c564483d1b20dc61834fa94fb8d42a6c9d00fd50729f35c31d441c513d").unwrap();
        assert_eq!(player.unknown_reason.as_deref(),Some("hidden-callable-implementation"));
        assert!(bound.storage_gaps(Some(&player.registration_id)).contains(&StorageGap::GetterImplementationUnproven));
        let observed=bound.observation.getters.iter().find(|g|g.state=="observed").unwrap();
        assert_eq!(bound.storage_gaps(Some(&observed.registration_id)),[StorageGap::GenerationUnproven,StorageGap::ProducerOriginUnproven]);
        assert!(serde_json::from_value::<super::super::CaptureRegistry>(manifest["registryCapture"].clone()).is_err(),
            "capture-local boundary observations cannot be cast to independent generation/origin evidence");
    }
}

/// Investigation only. Padding isolates total byte transfer/JSON cost while
/// keeping the real registry's record counts fixed; it does not model addon
/// semantics or certify the padded document. No production limit is changed.
#[test]
#[ignore = "bounded offline size investigation; run explicitly with --nocapture"]
fn measure_manifest_byte_scaling() {
    use std::time::Instant;
    use crate::storage::document::{self,Chunk};
    let original=include_bytes!("data/real-capture/reflect-manifest.json");
    let types:Vec<Value>=serde_json::from_str(include_str!("data/real-capture/reflect-types.json")).unwrap();
    let events:Vec<Value>=serde_json::from_str(include_str!("data/real-capture/reflect-event-values.json")).unwrap();
    let original_value:Value=serde_json::from_slice(original).unwrap();
    let source=SourceIdentity {snapshot_id:original_value["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
    for target in [original.len(),1024*1024,2*1024*1024,4*1024*1024] {
        let data=if target==original.len() {original.to_vec()} else {
            let mut value=original_value.clone();value["measurementPadding"]=json!("");
            let length=serde_json::to_vec(&value).unwrap().len();assert!(length<target);
            value["measurementPadding"]=json!("x".repeat(target-length));
            let bytes=serde_json::to_vec(&value).unwrap();assert_eq!(bytes.len(),target);bytes
        };
        let mut read_us=Vec::new();let mut parse_us=Vec::new();let mut projection_us=Vec::new();let mut serde_only_us=Vec::new();let mut read_calls=0;
        for _ in 0..5 {
            let start=Instant::now();let mut calls=0;
            let read=document::read_bound(&source,document::MANIFEST_LIMIT,||Ok(source.clone()),|offset,max|{
                calls+=1;let start=offset as usize;let end=data.len().min(start+max as usize);
                Ok(Some(Chunk {offset,total_length:data.len() as u64,bytes:data[start..end].to_vec()}))
            });read_us.push(start.elapsed().as_micros() as u64);read_calls=calls;
            if target<=document::MANIFEST_LIMIT {
                let read=read.unwrap().unwrap();let start=Instant::now();
                let bound=BoundObservation::parse(&source,&read,&source).unwrap().unwrap();parse_us.push(start.elapsed().as_micros() as u64);
                let start=Instant::now();assert_eq!(bound.check_projection(&types,&events),Ok(()));projection_us.push(start.elapsed().as_micros() as u64);
            } else {assert!(matches!(read,Err(document::ReadFailure::Limit)));}
            // A separately measured bounded stdlib deserialize at 4MiB is not
            // acceptance by the production reader/observation contract.
            let start=Instant::now();let parsed:Value=serde_json::from_slice(&data).unwrap();std::hint::black_box(&parsed);serde_only_us.push(start.elapsed().as_micros() as u64);
        }
        eprintln!("storage-input-size-measurement {}",json!({"bytes":target,"iterations":5,"profile":"native test opt1","recordCountsFixed":true,
            "readerLimitBytes":document::MANIFEST_LIMIT,"chunkBytes":document::CHUNK_BYTES,"readCalls":read_calls,"productionReadAccepted":target<=document::MANIFEST_LIMIT,
            "readMicros":read_us,"observationParseMicros":parse_us,"projectionMicros":projection_us,"serdeOnlyMicros":serde_only_us,
            "guestFuelMeasured":false,"heapMeasured":false,"readerBufferUpperBoundBytes":target.min(document::MANIFEST_LIMIT)}));
    }
}

#[test]
fn trusted_manifest_token_keeps_name_source_and_unbound_bytes_separate() {
    use crate::storage::document::{self,Chunk};
    let bytes=include_bytes!("data/real-capture/core-manifest.json");
    let manifest:Value=serde_json::from_slice(bytes).unwrap();
    let source=SourceIdentity {snapshot_id:manifest["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
    let make=|name|document::mock_document(&source,name,||Ok(source.clone()),|offset,max|{
        let start=offset as usize;let end=bytes.len().min(start+max as usize);
        Ok(Some(Chunk {offset,total_length:bytes.len() as u64,bytes:bytes[start..end].to_vec()}))
    }).unwrap().unwrap();
    let token=make("Manifest.json");let (_,observation)=BoundObservation::split_bound_manifest(&token,&source).unwrap();
    let bound=observation.unwrap().unwrap();assert!(bound.matches_source(&source));
    assert!(!bound.matches_current(&source,bytes),"arbitrary bytes cannot use a host-only fence");
    let types:Vec<Value>=serde_json::from_str(include_str!("data/real-capture/core-types.json")).unwrap();
    let events:Vec<Value>=serde_json::from_str(include_str!("data/real-capture/core-event-values.json")).unwrap();
    assert_eq!(bound.check_projection(&types,&events),Ok(()));
    assert_eq!(BoundObservation::split_bound_manifest(&make("Other.json"),&source).err(),Some(Failure::Source));
    let mut changed=source.clone();changed.source_digest="b".repeat(64);
    assert_eq!(BoundObservation::split_bound_manifest(&token,&changed).err(),Some(Failure::Source));
}
