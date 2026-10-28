use super::*;
use serde_json::json;

const BASE: &str = "ch.njol.skript.classes.ClassInfo";

fn identity(id: &str, name: &str, kind: &str, ancestors: &[&str]) -> JavaIdentity {
    JavaIdentity { id:id.into(), name:name.into(), kind:kind.into(), loader_id:"fixture-loader".into(),
        origin_sha256:"c".repeat(64), ancestry_complete:true, ancestors:ancestors.iter().map(|s|(*s).into()).collect() }
}

fn fixture() -> (CaptureRegistry, Vec<Value>, ProducerClaim) {
    let source = SourceIdentity { snapshot_id:"a".repeat(64), source_digest:"b".repeat(64) };
    let classes = vec![identity("object","Object","Class",&[]), identity("public","Public","Interface",&[]),
        identity("native","Native","Class",&["object","public"]), identity("other","Other","Class",&["object"]),
        identity("slot","Slot","Class",&["object"]), identity("info","Info","Class",&["object"])];
    let entries = ["other","public","object"].iter().enumerate().map(|(i,id)| RegistrationBinding {
        registration_id:format!("type:{i}"), class_id:(*id).into(), class_info_class_id:"info".into(),
        class_info_instance:format!("instance:{i}"), exact_class_info_instance:format!("instance:{i}"),
    }).collect::<Vec<_>>();
    let records = entries.iter().enumerate().map(|(i,entry)| {
        let class = classes.iter().find(|c|c.id==entry.class_id).unwrap();
        let ancestors = class.ancestors.iter().map(|id|classes.iter().find(|c|c.id==*id).unwrap().name.as_str()).collect::<Vec<_>>();
        json!({"originalClass":class.name,"registrationId":entry.registration_id,"typeParseOrder":i,
            "typeRuntime":{"version":3,"classInfoClass":"Info","classGetterDeclaringClass":BASE,
                "classState":"present","classValue":class.name,"classKind":class.kind,
                "ancestryComplete":true,"ancestors":ancestors,"cloneMethodState":"known",
                "cloneMethodDeclaringClass":BASE,"clonerGetterDeclaringClass":BASE,"clonerState":"absent",
                "serializeAsGetterDeclaringClass":BASE,"serializeAsState":"absent"}})
    }).collect();
    let capture = CaptureRegistry { version:1, source:source.clone(), generation_start:7, generation_end:7,
        complete:true, entries, classes };
    let subject = ValueSubject { source, generation:7, analysis_epoch:12, owner:"fixture.provider".into(),
        producer_registration_id:"expression:producer".into(), scope:ScopeSubject {
            document:"file:///public-fixture.sk".into(),revision:4,syntax_context:3,expansion:Some(9),start:24,end:31 } };
    let claim = ProducerClaim {version:1, subject, public_bound:"Public".into(), implementation_upper_bound:"Native".into(),
        complete:true,single:true,maybe_unset:true,before_change:"default-clone".into(),
        origin:Some(OriginClaim {source_model:"fixture-source-model".into(),artifact_sha256:"d".repeat(64),route:"native-route".into()})};
    (capture,records,claim)
}

fn failure(capture: &CaptureRegistry, records: &[Value]) -> Option<Failure> {
    capture.check(&SourceIdentity {snapshot_id:"a".repeat(64),source_digest:"b".repeat(64)},7,records).err()
}

#[test]
fn complete_synthetic_contract_gives_only_a_conditional_domain_result() {
    let (capture, records, mut claim) = fixture();
    let checked = capture.check(&claim.subject.source,7,&records).unwrap();
    let result = checked.evaluate(&claim,&claim.subject,"Slot").unwrap();
    assert_eq!(result,ConditionalStorage {public_bound:"Public".into(),implementation_upper_bound:"Native".into(),maybe_unset:true});
    claim.maybe_unset=false;
    assert!(!checked.evaluate(&claim,&claim.subject,"Slot").unwrap().maybe_unset);
    // No ResultType, effect assertion, metadata or seal is produced by this API.
}

#[test]
fn collection_identity_completeness_generation_and_limits_fail_closed() {
    let (capture,records,_) = fixture();
    for field in ["version","generationStart","generationEnd","complete","entries","source"] {
        let mut json=serde_json::to_value(&capture).unwrap();
        match field {
            "version"=>json[field]=json!(2),
            "generationStart"|"generationEnd"=>json[field]=json!(8),
            "complete"=>json[field]=json!(false),
            "entries"=>{json[field].as_array_mut().unwrap().pop();},
            _=>json[field]["sourceDigest"]=json!("e".repeat(64)),
        }
        let altered:CaptureRegistry=serde_json::from_value(json).unwrap();
        assert!(failure(&altered,&records).is_some(),"{field}");
    }
    assert_eq!(capture.check(&capture.source,0,&records).err(),Some(Failure::Generation));
    let mut empty=capture.clone();empty.classes.clear();
    assert_eq!(failure(&empty,&records),Some(Failure::Limit));
    assert_eq!(failure(&capture,&[]),Some(Failure::Incomplete));
    let mut oversized=capture.clone();oversized.classes[0].id="x".repeat(1025);
    assert_eq!(failure(&oversized,&records),Some(Failure::Identity));
    let mut extended=serde_json::to_value(&capture).unwrap();extended["trusted"]=json!(true);
    assert!(serde_json::from_value::<CaptureRegistry>(extended).is_err(),"unknown trust flags are not a contract");
    assert_eq!(CaptureRegistry::parse(&vec![b' ';1024*1024+1]).err(),Some(Failure::Limit));
    assert!(CaptureRegistry::parse(b"{broken").is_err());
    assert!(CaptureRegistry::parse(&serde_json::to_vec(&capture).unwrap()).is_ok());
}

#[test]
fn exact_instance_binding_order_and_getter_are_not_inferred_from_labels() {
    let (capture,records,_) = fixture();
    let mut wrong=capture.clone();wrong.entries[0].exact_class_info_instance="another-instance".into();
    assert_eq!(failure(&wrong,&records),Some(Failure::Routing));
    wrong=capture.clone();wrong.entries.swap(0,1);
    assert_eq!(failure(&wrong,&records),Some(Failure::Routing));
    wrong=capture.clone();wrong.entries[1].class_info_instance=wrong.entries[0].class_info_instance.clone();
    wrong.entries[1].exact_class_info_instance=wrong.entries[1].class_info_instance.clone();
    assert_eq!(failure(&wrong,&records),Some(Failure::Routing));
    for (field,value) in [("classGetterDeclaringClass",json!("CustomGetter")),("version",json!(2)),
        ("classInfoClass",json!("DifferentInfo")),("classValue",json!("DifferentClass"))] {
        let mut wrong_records=records.clone();wrong_records[0]["typeRuntime"][field]=value;
        assert!(failure(&capture,&wrong_records).is_some(),"{field}");
    }
    let mut wrong_records=records.clone();wrong_records[1]["registrationId"]=json!("different-registration");
    assert_eq!(failure(&capture,&wrong_records),Some(Failure::Routing));
    wrong_records=records.clone();wrong_records[1]["typeParseOrder"]=json!(0);
    assert_eq!(failure(&capture,&wrong_records),Some(Failure::Routing));
}

#[test]
fn missing_shadowed_and_inconsistent_java_identities_are_unknown() {
    let (capture,records,_) = fixture();
    for change in ["duplicateId","sameNameOtherLoader","missingAncestor","cycle","incomplete","origin","runtimeAncestry","multipleClassParents"] {
        let mut c=capture.clone();let mut r=records.clone();
        match change {
            "duplicateId"=>c.classes.push(c.classes[0].clone()),
            "sameNameOtherLoader"=>{let mut shadow=c.classes[0].clone();shadow.id="shadow".into();shadow.loader_id="other-loader".into();c.classes.push(shadow);},
            "missingAncestor"=>c.classes[2].ancestors.push("missing".into()),
            "cycle"=>c.classes[0].ancestors.push("native".into()),
            "incomplete"=>c.classes[2].ancestry_complete=false,
            "origin"=>c.classes[0].origin_sha256="unknown".into(),
            "multipleClassParents"=>c.classes[2].ancestors.extend(["other".into(),"slot".into()]),
            _=>r[0]["typeRuntime"]["ancestors"]=json!([]),
        }
        assert!(failure(&c,&r).is_some(),"{change}");
    }
}

#[test]
fn producer_origin_public_upper_and_effect_contracts_stay_separate() {
    let (capture,records,claim) = fixture();
    let checked=capture.check(&capture.source,7,&records).unwrap();
    let mut changed=claim.clone();changed.origin=None;
    assert_eq!(checked.evaluate(&changed,&claim.subject,"Slot"),Err(Failure::Origin));
    for field in ["sourceModel","artifactSha256","route"] {
        let mut changed=claim.clone();let origin=changed.origin.as_mut().unwrap();
        match field {"sourceModel"=>origin.source_model.clear(),"artifactSha256"=>origin.artifact_sha256.clear(),_=>origin.route.clear()}
        assert_eq!(checked.evaluate(&changed,&claim.subject,"Slot"),Err(Failure::Origin));
    }
    for change in ["version","complete","multiple","beforeChange","publicAsUpper","unknownUpper"] {
        let mut changed=claim.clone();
        match change {"version"=>changed.version=2,"complete"=>changed.complete=false,"multiple"=>changed.single=false,
            "beforeChange"=>changed.before_change="custom-callback".into(),"publicAsUpper"=>changed.implementation_upper_bound="Public".into(),
            _=>changed.implementation_upper_bound="Missing".into()}
        assert!(checked.evaluate(&changed,&claim.subject,"Slot").is_err(),"{change}");
    }
    assert_eq!(checked.evaluate(&claim,&claim.subject,"missing-slot"),Err(Failure::Storage));
}

#[test]
fn current_subject_rejects_stale_owner_scope_revision_and_source() {
    let (capture,records,claim) = fixture();
    let checked=capture.check(&capture.source,7,&records).unwrap();
    for field in ["owner","producer","revision","document","context","expansion","start","end","source","snapshot","generation","analysisEpoch"] {
        let mut current=claim.subject.clone();
        match field {
            "owner"=>current.owner="different.provider".into(),"producer"=>current.producer_registration_id="different-producer".into(),
            "revision"=>current.scope.revision+=1,"document"=>current.scope.document.push('x'),"context"=>current.scope.syntax_context+=1,
            "expansion"=>current.scope.expansion=None,"start"=>current.scope.start+=1,"end"=>current.scope.end+=1,
            "source"=>current.source.source_digest="e".repeat(64),"snapshot"=>current.source.snapshot_id="f".repeat(64),
            "analysisEpoch"=>current.analysis_epoch+=1,_=>current.generation+=1,
        }
        assert_eq!(checked.evaluate(&claim,&current,"Slot"),Err(Failure::Subject),"{field}");
        let mut forged=claim.clone();forged.subject=current.clone();
        if matches!(field,"source"|"snapshot"|"generation") {
            assert_eq!(checked.evaluate(&forged,&current,"Slot"),Err(Failure::Subject),"matching stale subjects do not change the checked capture");
        }
    }
}

#[test]
fn unrelated_custom_clone_is_excluded_but_related_and_late_exact_are_checked() {
    let (capture,records,claim) = fixture();
    let mut changed=records.clone();changed[0]["typeRuntime"]["clonerState"]=json!("present");
    assert!(capture.check(&capture.source,7,&changed).unwrap().evaluate(&claim,&claim.subject,"Slot").is_ok());
    changed=records.clone();changed[1]["typeRuntime"]["clonerState"]=json!("present");
    assert_eq!(capture.check(&capture.source,7,&changed).unwrap().evaluate(&claim,&claim.subject,"Slot"),Err(Failure::Storage));
    let mut capture=capture;
    capture.classes.push(identity("child","Child","Class",&["object","public","native"]));
    let mut child=records[0].clone();child["originalClass"]=json!("Child");child["registrationId"]=json!("type:3");child["typeParseOrder"]=json!(3);
    child["typeRuntime"]["classValue"]=json!("Child");child["typeRuntime"]["ancestors"]=json!(["Object","Public","Native"]);
    child["typeRuntime"]["clonerState"]=json!("present");
    capture.entries.push(RegistrationBinding {registration_id:"type:3".into(),class_id:"child".into(),class_info_class_id:"info".into(),class_info_instance:"instance:3".into(),exact_class_info_instance:"instance:3".into()});
    let mut records=records;records.push(child);
    assert_eq!(capture.check(&capture.source,7,&records).unwrap().evaluate(&claim,&claim.subject,"Slot"),Err(Failure::Storage));
}

#[test]
fn scope_coverage_keeps_current_byte_coordinates_and_nonempty_boundaries() {
    let (_,_,claim)=fixture();let parent=claim.subject.scope;
    assert!(parent.covers(&parent));
    let mut inner=parent.clone();inner.start+=1;inner.end-=1;
    assert!(parent.covers(&inner));
    inner.start=inner.end;assert!(!parent.covers(&inner));
    inner=parent.clone();inner.start-=1;assert!(!parent.covers(&inner));
    inner=parent.clone();inner.end+=1;assert!(!parent.covers(&inner));
    inner=parent.clone();inner.revision+=1;assert!(!parent.covers(&inner));
    // No byte/UTF-16 conversion is inferred here; native source maps own it.
}
