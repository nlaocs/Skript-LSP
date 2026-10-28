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
