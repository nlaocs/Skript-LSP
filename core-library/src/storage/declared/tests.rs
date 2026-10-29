use super::*;
use serde_json::json;
fn d(name:&str,parent:Option<&str>,interfaces:&[&str])->Declaration {
    Declaration {name:name.into(),kind:"Class".into(),super_class:parent.map(str::to_owned),interfaces:interfaces.iter().map(|s|s.to_string()).collect()}
}
fn row(c:&str,i:usize)->Value {json!({"originalClass":c,"typeParseOrder":i,"typeRuntime":{
    "version":3,"classState":"present","classGetterDeclaringClass":BASE,"classValue":c,
    "cloneMethodState":"known","cloneMethodDeclaringClass":BASE,"clonerGetterDeclaringClass":BASE,"clonerState":"absent",
    "serializeAsGetterDeclaringClass":BASE,"serializeAsState":"absent"}})}
fn model()->Model {
    let declarations=vec![d("java.lang.Object",None,&[]),d("custom.Base",Some("java.lang.Object"),&[]),d("custom.Child",Some("custom.Base"),&[]),
        d("custom.Target",Some("java.lang.Object"),&[]),d(SLOT,Some("java.lang.Object"),&[]),d(ITEM,Some("java.lang.Object"),&[])];
    let records=["custom.Base",SLOT,ITEM,"java.lang.Object","custom.Child","custom.Target"].iter().enumerate().map(|(i,c)|row(c,i)).collect();
    Model::new(records,declarations).unwrap()
}
fn infer(m:&Model,c:&str)->Inference {m.infer(&[c.into()]).unwrap()}
#[test]
fn previously_unknown_classes_use_public_bounds_without_native_producer_or_hashes() {
    let m=model();let r=infer(&m,"custom.Child");
    assert_eq!(r.types,["custom.Child"]);assert!(r.complete&&r.maybe_unset&&r.preserves_local_bindings);
    assert_eq!(r.basis,BASIS);assert!(!serde_json::to_string(&r).unwrap().contains("native"));
}
#[test]
fn inherited_hook_must_be_default_through_the_entire_known_closure() {
    let mut m=model();
    m.declarations.insert("custom.Getter".into(),d("custom.Getter",Some("custom.Parent"),&[]));
    m.declarations.insert("custom.Parent".into(),d("custom.Parent",Some("java.lang.Object"),&[EXPRESSION]));
    let mut expr=d(EXPRESSION,None,&[]);expr.kind="Interface".into();m.declarations.insert(EXPRESSION.into(),expr);
    assert!(m.default_before_change("custom.Getter",|c|Some(c==EXPRESSION)));
    assert!(!m.default_before_change("custom.Getter",|c|Some(c==EXPRESSION||c=="custom.Parent")));
    assert!(!m.default_before_change("custom.Getter",|c|(c!="custom.Parent").then_some(c==EXPRESSION)));
    m.declarations.remove("custom.Parent");assert!(!m.default_before_change("custom.Getter",|c|Some(c==EXPRESSION)));
}
#[test]
fn cloner_widens_to_its_declared_class_then_dispatches_storage_again() {
    let mut m=model();m.records[0]["typeRuntime"]["clonerState"]="present".into();
    m.records.remove(4);
    for (i,r) in m.records.iter_mut().enumerate() {r["typeParseOrder"]=json!(i);}
    m.records[0]["serializeAs"]=json!("custom.Target");m.records[0]["typeRuntime"]["serializeAsState"]="present".into();
    m.records[0]["typeRuntime"]["serializeAsClass"]=json!("custom.Target");
    let r=infer(&m,"custom.Child");
    assert!(r.types.contains(&"custom.Target".into()));assert!(!r.preserves_local_bindings);
    assert!(!r.types.contains(&"custom.Child".into()),"RHS is not blindly copied");
}
#[test]
fn exact_registration_after_covering_ordered_entry_is_not_lost() {
    let mut m=model();m.records[4]["typeRuntime"]["clonerState"]="present".into();
    m.records[4]["serializeAs"]=json!("custom.Target");m.records[4]["typeRuntime"]["serializeAsState"]="present".into();
    m.records[4]["typeRuntime"]["serializeAsClass"]=json!("custom.Target");
    assert!(infer(&m,"custom.Base").types.contains(&"custom.Target".into()));
}
#[test]
fn serialization_uses_the_selected_dispatch_class_without_losing_real_conversions() {
    for (target,changes_type) in [("custom.Base",false),("custom.Target",true)] {
        let mut m=model();
        let mut interface=d("custom.Interface",None,&["java.lang.Object"]);interface.kind="Interface".into();
        let mut declarations=m.declarations.values().cloned().collect::<Vec<_>>();
        declarations.iter_mut().find(|d|d.name=="custom.Child").unwrap().interfaces.push(interface.name.clone());
        declarations.push(interface);
        m.records[4]["serializeAs"]=json!(target);
        m.records[4]["typeRuntime"]["serializeAsState"]="present".into();
        m.records[4]["typeRuntime"]["serializeAsClass"]=json!(target);
        let m=Model::new(m.records,declarations).unwrap();
        let (output,preserves)=m.stored_bounds("custom.Interface").unwrap();
        assert_eq!(output.contains(target),changes_type,"{target}: {output:?}");
        assert!(output.contains("custom.Interface"));
        assert_eq!(preserves,!changes_type);
    }
}
#[test]
fn slots_convert_and_open_interfaces_include_possible_slot_branch() {
    let mut m=model();assert_eq!(infer(&m,SLOT).types,[ITEM]);
    let mut interface=d("custom.Interface",None,&["java.lang.Object"]);interface.kind="Interface".into();
    // A public interface cannot be treated as a concrete implementation domain.
    let declarations=m.declarations.values().cloned().chain([interface]).collect();
    m=Model::new(m.records,declarations).unwrap();
    assert!(infer(&m,"custom.Interface").types.contains(&ITEM.into()));
}
#[test]
fn incomplete_unknown_and_old_metadata_do_not_gain_transfer_or_purity() {
    let mut m=model();m.records[0]["typeRuntime"]["serializeAsState"]="unknown".into();assert!(m.infer(&["custom.Child".into()]).is_none());
    let mut m=model();m.records[0]["typeRuntime"]["cloneMethodState"]="unknown".into();assert!(m.infer(&["custom.Child".into()]).is_none());
    let m=model();let mut records=m.records.clone();records[0]["typeRuntime"]["version"]=json!(1);
    assert!(Model::new(records,m.declarations.into_values().collect()).is_none());
    for classes in [vec![],vec!["java.lang.Object".into()],vec!["missing.Type".into()]] {assert!(model().infer(&classes).is_none());}
}
#[test]
fn versions_are_semantic_adapters_not_artifact_allowlists() {
    for version in ["2.12.2","2.15.4","2.15.5-custom"] {assert!(supports(version));}
    for version in ["2.6.4","2.11.0","2.16.0","unknown"] {assert!(!supports(version));}
}

#[test]
fn captured_standard_type_rows_work_without_pinned_native_environment() {
    let rows=serde_json::from_str(include_str!("../../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/Types.json")).unwrap();
    let model=Model::from_records(rows).unwrap();
    assert_eq!(infer(&model,"java.lang.String").types,["java.lang.String"]);
    // A concrete registered class absent from all producer allowlists uses the
    // same type-contract transfer. UUID has no per-event producer rule.
    assert_eq!(infer(&model,"java.util.UUID").types,["java.util.UUID"]);
}

#[test]
fn complete_method_closure_has_one_standard_hook_and_no_unknown_override() {
    let closure=["owned.Getter".into(),EXPRESSION.into(),"java.lang.Object".into()];
    assert!(default_hook(&closure,|c|Some(c==EXPRESSION)));
    assert!(!default_hook(&closure,|c|Some(c==EXPRESSION||c=="owned.Getter")));
    assert!(!default_hook(&closure,|c|(c!="owned.Getter").then_some(c==EXPRESSION)));
    assert!(!default_hook(&closure[..1],|_|Some(false)));
}

#[test]
fn optional_native_step_budget_cannot_report_an_unpublished_model_ready() {
    assert!(source::budget_label(true).starts_with("declared-storage-ready;"));
    assert!(source::budget_label(false).starts_with("declared-storage-unavailable;"));
}

#[test]
fn delegated_standard_hook_rejects_unknown_or_additional_overrides() {
    let forwarder="standard.Wrapper";
    let closure=["standard.Factory".into(),forwarder.into(),EXPRESSION.into(),"java.lang.Object".into()];
    assert!(forwarding_hook(&closure,forwarder,|c|Some(c==forwarder||c==EXPRESSION)));
    assert!(!default_hook(&closure,|c|Some(c==forwarder||c==EXPRESSION)),"forwarding is not default dispatch");
    assert!(!forwarding_hook(&closure,forwarder,|c|Some(c==forwarder||c==EXPRESSION||c=="standard.Factory")));
    assert!(!forwarding_hook(&closure,forwarder,|c|(c!="standard.Factory").then_some(c==forwarder||c==EXPRESSION)));
    assert!(!forwarding_hook(&closure,"other.Wrapper",|c|Some(c==forwarder||c==EXPRESSION)));
    // A legitimate outer forwarding path does not authorize a receiver override.
    let receiver=["custom.Receiver".into(),EXPRESSION.into()];
    assert!(!default_hook(&receiver,|_|Some(true)));
}

#[test]
fn unsupported_array_geometry_and_missing_storage_targets_fail_closed() {
    let captured=include_str!("../../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/Types.json");
    for kind in ["Array","Primitive"] {
        let mut rows:Vec<Value>=serde_json::from_str(captured).unwrap();
        rows[0]["typeRuntime"]["classKind"]=kind.into();
        assert!(Model::from_records(rows).is_none());
    }
    let mut m=model();m.records[4]["serializeAs"]=json!("missing.Target");
    m.records[4]["typeRuntime"]["serializeAsState"]="present".into();
    m.records[4]["typeRuntime"]["serializeAsClass"]="missing.Target".into();
    assert!(m.infer(&["custom.Child".into()]).is_none());
}

#[test]
fn contradictory_clone_observations_cannot_certify_identity_or_purity() {
    let mut m=model();m.records[4]["typeRuntime"]["clonerClass"]="custom.Cloner".into();
    assert!(!infer(&m,"custom.Child").preserves_local_bindings);
    let mut m=model();m.records[4]["typeRuntime"]["clonerUnknownReason"]="unavailable".into();
    assert!(!infer(&m,"custom.Child").preserves_local_bindings);
    m.records[4]["typeRuntime"]["cloneMethodUnknownReason"]="unavailable".into();
    assert!(m.infer(&["custom.Child".into()]).is_none());
}
