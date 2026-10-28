use super::*;
use serde_json::json;

pub(in crate::storage) const COLLECTOR:&str="0dd2ae968f2f646283dbfdda30dcb1cc78e8ae86182fc9fe5134c4bed006ebc2";

/// Deliberately simulated new observations over an immutable old-data copy.
/// This is NOT recaptured evidence and is never installed into a user snapshot.
pub(in crate::storage) fn synthetic()->(SourceIdentity,Value,Vec<Value>,Vec<Value>) {
    let mut m:Value=serde_json::from_slice(include_bytes!("../data/real-capture/reflect-manifest.json")).unwrap();
    let types=serde_json::from_slice(include_bytes!("../data/real-capture/reflect-types.json")).unwrap();
    let events=serde_json::from_slice(include_bytes!("../data/real-capture/reflect-event-values.json")).unwrap();
    for plugin in m["plugins"].as_array_mut().unwrap(){if plugin["name"]=="SkriptSyntaxGenerator"{plugin["jarSha256"]=json!(COLLECTOR);}}
    let o=&mut m["registryCapture"];o["version"]=json!(2);o["generationEvidence"]=json!("supported-api-interval-with-quiescent-premise-v1");
    let base=o["classes"].as_array().unwrap().iter().find(|c|c["name"]==BASE).unwrap().clone();
    let loader=base["loaderId"].as_str().unwrap().to_owned();
    fn class(o:&mut Value,name:&str,sha:&str,loader:&str)->String {
        if let Some(row)=o["classes"].as_array().unwrap().iter().find(|r|r["name"]==name){return row["id"].as_str().unwrap().to_owned();}
        let id=format!("owned-simulated-class-{}",o["classes"].as_array().unwrap().len());
        o["classes"].as_array_mut().unwrap().push(json!({"id":id,"name":name,"catalogName":name,"kind":"Class","loaderId":loader,
            "codeOriginSha256":sha,"originState":"code-source-file","hidden":false,"ancestryComplete":true,"ancestors":[]}));id
    }
    let factory=class(o,FACTORY,SKRIPT,&loader);let registry=class(o,REGISTRY,SKRIPT,&loader);let caller=class(o,CALLER,SKRIPT,&loader);
    let classes=class(o,"ch.njol.skript.registrations.Classes",SKRIPT,&loader);
    let event=o["classes"].as_array().unwrap().iter().find(|c|c["name"]==EVENT).unwrap()["id"].as_str().unwrap().to_owned();
    let value=o["classes"].as_array().unwrap().iter().find(|c|c["name"]==VALUE).unwrap()["id"].as_str().unwrap().to_owned();
    let stamp=json!({"lifetime":1,"sequence":2,"coverageSequence":0,"inFlight":0,"incomplete":false});
    o["interval"]=json!({"version":1,"scope":"synchronous-export-supported-api","premise":"initialized-quiescent-registry","registryInstanceId":"owned-registry",
        "before":stamp,"after":stamp,"typeDetailsBefore":"d".repeat(64),"typeDetailsAfter":"d".repeat(64),"eventValuesBefore":"e".repeat(64),"eventValuesAfter":"e".repeat(64),"complete":true,
        "coverage":[{"classId":base["id"],"methods":["cloner(Lch/njol/skript/classes/Cloner;)Lch/njol/skript/classes/ClassInfo;","serializeAs(Ljava/lang/Class;)Lch/njol/skript/classes/ClassInfo;","user([Ljava/lang/String;)Lch/njol/skript/classes/ClassInfo;"]},
        {"classId":factory,"methods":[SIMPLE]},{"classId":registry,"methods":[REGISTER,UNREGISTER]},
        {"classId":classes,"methods":["registerClass(Lch/njol/skript/classes/ClassInfo;)V","onRegistrationsStop()V"]}]});
    o["factorySources"]=json!([{"id":"source","classId":caller,"classSha256":"c".repeat(64),"artifactSha256":SKRIPT}]);
    o["factoryMethods"]=json!([{"id":"method","sourceId":"source","name":"register","descriptor":"(Lorg/skriptlang/skript/bukkit/lang/eventvalue/EventValueRegistry;)V"}]);
    o["factorySites"]=json!([{"id":"site","methodId":"method","bytecodeIndex":1,"eventClassId":event,"valueClassId":value,"factoryClassId":factory,
        "targetClassId":event,"targetName":"getPlayer","targetDescriptor":"()Lorg/bukkit/entity/Player;","referenceKind":5,"targetInterface":false,
        "registrationMethodId":"method","registrationBytecodeIndex":2,"registryClassId":registry}]);
    for (index,getter) in o["getters"].as_array_mut().unwrap().iter_mut().enumerate(){
        getter["eventValueInstanceId"]=json!(format!("owned-event-value-{index}"));
        if getter["eventClassId"]==event&&getter["valueClassId"]==value{getter["factorySiteId"]=json!("site");}
    }
    let source=SourceIdentity {snapshot_id:m["snapshotId"].as_str().unwrap().into(),source_digest:"f".repeat(64)};
    (source,m,types,events)
}
fn bound(source:&SourceIdentity,m:&Value)->BoundObservation {BoundObservation::parse(source,&serde_json::to_vec(m).unwrap(),source).unwrap().unwrap()}

#[test]
fn simulated_v2_qualifies_only_a_pinned_finite_native_target_not_a_runtime_epoch() {
    let (source,m,types,events)=synthetic();let b=bound(&source,&m);b.check_projection(&types,&events).unwrap();
    let target=b.standard_native_target(&m,COLLECTOR).unwrap();assert!(!target.getter.is_empty());
    let mut extra=m.clone();extra["plugins"].as_array_mut().unwrap().push(json!({"name":"UnrelatedOwnedAddon","enabled":true,"jarSha256":"a".repeat(64)}));
    assert!(bound(&source,&extra).standard_native_target(&extra,COLLECTOR).is_some(),"plugin count is not a proof boundary");
    let old:Value=serde_json::from_slice(include_bytes!("../data/real-capture/reflect-manifest.json")).unwrap();
    let old_source=SourceIdentity{snapshot_id:old["snapshotId"].as_str().unwrap().into(),source_digest:"a".repeat(64)};
    assert!(bound(&old_source,&old).standard_native_target(&old,COLLECTOR).is_none(),"v1 is not retroactively observed");
}

#[test]
fn changed_interval_source_site_loader_or_coverage_never_qualifies() {
    let (source,m,_,_)=synthetic();
    for mode in ["sequence","active","details","events","premise","target","site","caller","loader","coverage","collector","duplicate"] {
        let mut changed=m.clone();let o=&mut changed["registryCapture"];
        match mode {
            "sequence"=>o["interval"]["after"]["sequence"]=json!(3),
            "active"=>o["interval"]["before"]["inFlight"]=json!(1),
            "details"=>o["interval"]["typeDetailsAfter"]=json!("a".repeat(64)),
            "events"=>o["interval"]["eventValuesAfter"]=json!("a".repeat(64)),
            "premise"=>o["interval"]["premise"]=json!("all-deliveries-attested"),
            "target"=>o["factorySites"][0]["targetName"]=json!("customGetter"),
            "site"=>o["factorySites"][0]["methodId"]=json!("missing"),
            "caller"=>o["factoryMethods"][0]["descriptor"]=json!("()V"),
            "loader"=>{let id=o["factorySites"][0]["factoryClassId"].clone();let c=o["classes"].as_array_mut().unwrap().iter_mut().find(|c|c["id"]==id).unwrap();c["loaderId"]=json!("l0");},
            "coverage"=>o["interval"]["coverage"][0]["methods"]=json!([]),
            "collector"=>changed["plugins"][0]["jarSha256"]=json!("0".repeat(64)),
            _=>{let row=o["factorySites"][0].clone();o["factorySites"].as_array_mut().unwrap().push(row);}
        }
        let parsed=BoundObservation::parse(&source,&serde_json::to_vec(&changed).unwrap(),&source);
        assert!(parsed.is_err()||parsed.unwrap().is_none_or(|b|b.standard_native_target(&changed,COLLECTOR).is_none()),"{mode}");
    }
}

#[test]
fn java_emitted_tuple_roundtrips_whole_manifest_and_staged_raw_rows() {
    use crate::storage::{document,evidence::header};
    use crate::storage::evidence::observation::preparation::Builder;
    let f:Value=serde_json::from_str(include_str!("../data/ssg-observation-v2.json")).unwrap();
    let source=SourceIdentity{snapshot_id:"a".repeat(64),source_digest:"b".repeat(64)};
    let manifest=json!({"schemaVersion":7,"snapshotId":source.snapshot_id,"registryCapture":f["registryCapture"]});
    let raw=serde_json::to_vec(&manifest).unwrap();
    let bound=BoundObservation::parse(&source,&raw,&source).unwrap().unwrap();
    let types=f["types"].as_array().unwrap().clone();let events=f["eventValues"].as_array().unwrap();
    bound.check_projection(&types,events).unwrap();
    let wire:FactorySite=serde_json::from_str(f["siteBytes"].as_str().unwrap()).unwrap();
    assert_eq!(wire.id,"f0");assert_eq!(wire.bytecode_index,0);assert_eq!(wire.registration_bytecode_index,0);assert!(!wire.target_interface);
    let doc=document::mock_document(&source,"Manifest.json",||Ok(source.clone()),|offset,max|{
        let start=offset as usize;Ok(Some(document::Chunk{offset,total_length:raw.len()as u64,bytes:raw[start..raw.len().min(start+max as usize)].to_vec()}))
    }).unwrap().unwrap();
    let header=header::prepare(&doc,&source).unwrap().1.unwrap().unwrap();
    let mut builder=Builder::new(header).unwrap();
    for name in ["entries","exactBindings","classes","loaders","getters","factorySources","factoryMethods","factorySites"] {
        for rows in f["registryCapture"][name].as_array().unwrap().chunks(64){builder.append(name,rows.iter().map(|r|serde_json::to_vec(r).unwrap()).collect()).unwrap();}
    }
    builder.check_details().unwrap();builder.check_projection(types,events).unwrap();let projected=builder.finish().unwrap().into_bound();
    assert!(projected.standard_native_target(&manifest,COLLECTOR).is_none(),"owned structural fixture is not a certified Skript producer");
    let site:Value=serde_json::from_str(f["siteBytes"].as_str().unwrap()).unwrap();
    for mode in ["short","long","numeric","bool","identity-swap"] {
        let mut changed=site.clone();
        match mode {
            "short"=>{changed.as_array_mut().unwrap().pop();},
            "long"=>changed.as_array_mut().unwrap().push(Value::Null),
            "numeric"=>changed[2]=json!("0"),
            "bool"=>changed[10]=json!(0),
            _=>changed.as_array_mut().unwrap().swap(3,4),
        }
        let mut bad=manifest.clone();bad["registryCapture"]["factorySites"][0]=changed;
        assert!(BoundObservation::parse(&source,&serde_json::to_vec(&bad).unwrap(),&source).is_err(),"{mode}");
    }
}


#[test]
fn graph_name_projection_never_aliases_actual_class_or_loader_tokens() {
    let (source,m,_,_)=synthetic();
    for mode in ["entity-loader","slot-origin","ancestor-alias","source-node-alias"] {
        let mut changed=m.clone();let o=&mut changed["registryCapture"];
        let entity=o["classes"].as_array().unwrap().iter().position(|c|c["catalogName"]=="org.bukkit.entity.Entity").unwrap();
        let slot=o["classes"].as_array().unwrap().iter().position(|c|c["catalogName"]=="ch.njol.skript.util.slot.Slot").unwrap();
        match mode {
            "entity-loader"=>o["classes"][entity]["loaderId"]=json!("l0"),
            "slot-origin"=>o["classes"][slot]["codeOriginSha256"]=json!("a".repeat(64)),
            _=>{
                let mut alias=o["classes"][entity].clone();alias["id"]=json!("owned-alias");alias["loaderId"]=json!("l0");
                if mode=="source-node-alias" {alias["name"]=json!("org.bukkit.craftbukkit.entity.CraftEntity");alias["catalogName"]=alias["name"].clone();}
                o["classes"].as_array_mut().unwrap().push(alias);
                o["classes"][entity]["ancestors"].as_array_mut().unwrap().push(json!("owned-alias"));
            }
        }
        assert!(bound(&source,&changed).standard_native_target(&changed,COLLECTOR).is_none(),"{mode}");
    }
    let mut unrelated=m.clone();let o=&mut unrelated["registryCapture"];
    let mut unused=o["classes"].as_array().unwrap().iter().find(|c|c["catalogName"]=="org.bukkit.entity.Entity").unwrap().clone();
    unused["id"]=json!("owned-unused-class");unused["loaderId"]=json!("l0");o["classes"].as_array_mut().unwrap().push(unused);
    assert!(bound(&source,&unrelated).standard_native_target(&unrelated,COLLECTOR).is_some(),"unused rows are not graph dependencies");
}