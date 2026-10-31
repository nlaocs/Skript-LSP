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
fn callback_only_declared_input_still_runs_clone_and_serialization_transfer() {
    use crate::nlaocs::skript_parser_addon::types::*;
    let summary=ParseSummary {kind:"registered-expression".into(),definition_id:Some("custom.get".into()),registration_id:Some("custom.get.0".into()),
        element_class:Some("custom.Getter".into()),pattern_index:Some(0),return_type:Some("custom.Child".into()),possible_return_types:vec!["custom.Child".into()],
        possible_return_types_state:ExpressionPossibleReturnTypesState::Complete,multiplicity:Some(DynamicMultiplicity::Single),default_expression:None,
        public_data:vec![],metadata:vec![MetadataEntry {key:crate::storage::native_inputs::INPUT.into(),owner_component_id:None,value:json!({"version":1,"receipt":7,
            "subject":"a".repeat(64),"issuer":"foreign.provider","valueDomain":{"version":1,"classes":[],"complete":false,"maybeUnset":false,
            "beforeChange":"default-clone","evidence":"audited callback only"}}).to_string()}]};
    let bounds=crate::storage::native_inputs::declared_clone_bounds(&summary).unwrap();
    assert!(crate::storage::native_inputs::clone_classes(&summary).is_none());
    let mut m=model();let ordinary=m.infer(&bounds).unwrap();
    assert_eq!(ordinary.types,["custom.Child"]);assert!(ordinary.maybe_unset);
    m.records[0]["typeRuntime"]["clonerState"]="present".into();
    m.records.remove(4);
    for (i,r) in m.records.iter_mut().enumerate() {r["typeParseOrder"]=json!(i);}
    m.records[0]["serializeAs"]=json!("custom.Target");m.records[0]["typeRuntime"]["serializeAsState"]="present".into();
    m.records[0]["typeRuntime"]["serializeAsClass"]=json!("custom.Target");
    let converted=m.infer(&bounds).unwrap();
    assert!(converted.types.contains(&"custom.Target".into()));assert!(!converted.types.contains(&"custom.Child".into()));
    assert!(converted.maybe_unset);assert!(!converted.preserves_local_bindings);
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
    let closure=["owned.Getter".into(),"owned.Parent".into(),EXPRESSION.into(),"java.lang.Object".into()];
    assert!(default_hook(&closure,|c|Some(c==EXPRESSION)));
    for receiver in &closure {
        if receiver==EXPRESSION {continue;}
        assert!(!default_hook(&closure,|c|Some(c==EXPRESSION||c==receiver)),"override: {receiver}");
        assert!(!default_hook(&closure,|c|(c!=receiver).then_some(c==EXPRESSION)),"unknown: {receiver}");
    }
    assert!(!default_hook(&closure[..2],|_|Some(false)));
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

#[test]
fn resolved_single_pass_dispatch_matches_exact_then_ordered_reference() {
    fn old<'a>(m:&'a Model,bound:&str)->Option<Vec<&'a Value>> {
        if m.graph.assignable(bound,bound)!=Some(true)||!m.declarations.contains_key(bound){return None;}
        let mut indices=BTreeSet::new();
        for (i,r) in m.records.iter().enumerate() {
            let c=r["originalClass"].as_str()?;
            if m.graph.concrete(c)?&&m.graph.assignable(c,bound)?{indices.insert(i);}
        }
        let mut covered=false;
        for (i,r) in m.records.iter().enumerate() {
            let c=r["originalClass"].as_str()?;
            if m.graph.overlap(c,bound)?{indices.insert(i);}
            if m.graph.assignable(bound,c)?{covered=true;break;}
        }
        covered.then(||indices.into_iter().map(|i|&m.records[i]).collect())
    }
    let mut m=model();
    let mut interface=d("custom.Open",None,&["java.lang.Object"]);interface.kind="Interface".into();
    let mut late=d("custom.Late",None,&["missing.Parent"]);late.kind="Interface".into();
    let declarations=m.declarations.values().cloned().chain([
        interface,late,d("custom.Partial",Some("missing.Parent"),&[]) ]).collect();
    m=Model::new(m.records,declarations).unwrap();
    m.records.push(row("custom.Late",m.records.len()));
    let compare=|m:&Model|for bound in ["custom.Child","custom.Base","custom.Target","custom.Open",
        "custom.Partial","missing.Type",SLOT,ITEM,"java.lang.Object"] {
        let names=|rows:Option<Vec<&Value>>|rows.map(|rs|rs.into_iter().map(|r|r["originalClass"].as_str().unwrap().to_owned()).collect::<Vec<_>>());
        assert_eq!(names(m.dispatch(bound)),names(old(m,bound)),"{bound}");
    };
    compare(&m);
    // A late exact-class candidate must survive the first covering ordered row.
    let result=m.dispatch("custom.Base").unwrap();
    assert!(result.iter().any(|r|r["originalClass"]=="custom.Child"));
    assert!(!result.iter().any(|r|r["originalClass"]=="custom.Late"),"covered suffix interfaces are not newly queried");
    // An unavailable late registry class still invalidates the whole proof.
    m.records.push(row("missing.Row",m.records.len()));
    compare(&m);assert!(m.dispatch("custom.Child").is_none());
    m.records.insert(0,row("missing.Early",0));compare(&m);
}
