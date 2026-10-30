//! Flow-bound standard transfers. Operation semantics live in Core; the host
//! authenticates the selected operation and the frontend owns current flow.
use super::*;
use crate::exports::nlaocs::skript_parser_addon::semantic_transfer as wire;
use crate::nlaocs::skript_parser_addon::types::{MetadataEntry, ParseSummary};

const KEY: &str = "parser.semantic-transfer.v1";
const READ: &str = "skript.standard-variable-read.v1";
const SET: &str = "skript.standard-local-set.v1";
const GLOBAL: &str = "skript.standard-global-write-candidate.v1";
// The instanceof/replacement domain of Variable.convertIfOldPlayer in2.15.4.
// This describes the standard operation, not a permitted producer/type list.
const REFRESH_DOMAIN: &str = "org.bukkit.entity.Player";

#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Read {version:u32,lookup:String,requested_types:Vec<String>,no_refresh_types:Vec<String>}
#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Set {version:u32,kind:String}

fn operation(id:&str,parameters:String)->MetadataEntry {
    MetadataEntry {key:KEY.into(),value:serde_json::json!({"version":1,"operation":id,"parameters":parameters}).to_string(),owner_component_id:None}
}
pub(crate) fn read_operation(requested:&[String],local:bool)->Option<MetadataEntry> {
    (local && crate::runtime::skript_version_is("2.15.4") && requested==["java.lang.Object"])
        .then(||operation(READ,serde_json::to_string(&Read {version:1,lookup:"local-scalar-empty".into(),requested_types:requested.to_vec(),
            no_refresh_types:crate::catalog::identity_delta_classes()}).expect("read contract serializes")))
}
/// beforeChange belongs to the evaluated delta, not to EffChange. An arbitrary
/// addon override must not inherit the default clone/Slot/storage model.
pub(crate) fn set_operation(delta:&ParseSummary)->Option<MetadataEntry> {
    let mut entries=delta.metadata.iter().filter(|e|e.key==crate::public_data::CHANGE_INPUT_METADATA_KEY);
    let entry=entries.next()?;
    if entries.next().is_some() || entry.owner_component_id.as_deref()!=Some(crate::COMPONENT_ID) {return None;}
    let data:Value=serde_json::from_str(&entry.value).ok()?;
    (data==serde_json::json!({"version":1,"kind":"default-clone"}) && crate::runtime::skript_version_is("2.15.4"))
        .then(||operation(SET,serde_json::to_string(&Set {version:1,kind:"local-scalar-default-clone".into()}).expect("SET contract serializes")))
}
/// Upper bound of the value passed to global persistence, not a poststate.
pub(crate) fn global_set_operation(delta:&ParseSummary)->Option<MetadataEntry> {
    if !crate::runtime::skript_version_is("2.15.4") {return None;}
    if delta.metadata.iter().any(|entry|entry.key==crate::public_data::CHANGE_INPUT_METADATA_KEY) {
        // An explicit malformed/foreign/duplicate contract cannot fall back
        // to a different witness and silently become a valid default input.
        set_operation(delta)?;
    } else {
        // Registered expressions use the existing current-source, exact-summary
        // default-beforeChange witness. Its stored result authorizes the path;
        // it is NOT used as a new RHS (which would apply storage twice).
        super::for_summary(delta)?;
    }
    Some(MetadataEntry {key:KEY.into(),owner_component_id:None,value:serde_json::json!({"version":2,
        "outputUse":"completion-candidate","operation":GLOBAL,
        "parameters":serde_json::to_string(&Set {version:1,kind:"global-write-candidate-default-clone".into()}).ok()?}).to_string()})
}
fn valid(input:&wire::TypeBound)->bool {
    input.complete && !input.multiple && input.types.len()<=64 && (!input.types.is_empty()||input.maybe_unset)
        && input.types.iter().all(|s|!s.is_empty()&&s.len()<=1024&&s!="java.lang.Object")
        && input.types.iter().collect::<BTreeSet<_>>().len()==input.types.len()
}
fn bound(types:Vec<String>,maybe_unset:bool)->wire::TypeBound {
    wire::TypeBound {types,complete:true,multiple:false,maybe_unset}
}
fn effects(preserves:bool)->wire::BindingEffects {
    if preserves {wire::BindingEffects::Preserve}else{wire::BindingEffects::Invalidate}
}
pub(super) fn evaluate(model:&Model,request:&wire::Request)->Option<wire::Output> {
    if !valid(&request.input) {return None;}
    match request.operation.as_str() {
        READ=>{
            let parameters:Read=serde_json::from_str(&request.parameters).ok()?;
            if parameters.version!=1 || parameters.lookup!="local-scalar-empty" || parameters.requested_types!=["java.lang.Object"]
                || parameters.no_refresh_types.len()>64 {return None;}
            // Unknown enablePlayerVariableFix joins enabled and disabled paths.
            let mut refresh=false;
            for input in &request.input.types {
                if !parameters.no_refresh_types.contains(input) && model.graph.overlap(input,REFRESH_DOMAIN)? {refresh=true;}
            }
            if !refresh {
                return Some(wire::Output {value:request.input.clone(),source:wire::SourceUpdate::Unchanged,source_writeback:None,effects:wire::BindingEffects::Preserve});
            }
            let mut types=request.input.types.iter().cloned().collect::<BTreeSet<_>>();
            types.insert(REFRESH_DOMAIN.into());
            if types.len()>64 {return None;}
            let written=model.stored_bounds(REFRESH_DOMAIN).map(|(types,_)|bound(types.into_iter().collect(),true));
            // Refresh returns raw newPlayer. Its conditional writeback uses
            // serializeAs, without beforeChange. Callback effects on a no-write
            // branch prevent claiming a complete joined source poststate.
            Some(wire::Output {value:bound(types.into_iter().collect(),true),source:wire::SourceUpdate::Unknown,
                source_writeback:written,effects:wire::BindingEffects::Invalidate})
        },
        SET|GLOBAL=>{
            let parameters:Set=serde_json::from_str(&request.parameters).ok()?;
            let global=request.operation==GLOBAL;
            if parameters.version!=1 || parameters.kind!=if global {"global-write-candidate-default-clone"}else{"local-scalar-default-clone"} {return None;}
            if request.input.types.is_empty() {
                // An empty SET delta deletes the binding before cloning.
                return Some(wire::Output {value:bound(Vec::new(),true),source:if global {wire::SourceUpdate::Unknown}else{wire::SourceUpdate::Unchanged},source_writeback:None,effects:effects(!global)});
            }
            let stored=model.infer(&request.input.types)?;
            Some(wire::Output {value:bound(stored.types,request.input.maybe_unset||!stored.preserves_local_bindings&&stored.maybe_unset),
                source:if !global&&stored.preserves_local_bindings {wire::SourceUpdate::Unchanged}else{wire::SourceUpdate::Unknown},
                source_writeback:None,effects:effects(!global&&stored.preserves_local_bindings)})
        },
        _=>None,
    }
}
pub(crate) fn query(request:wire::Request)->Option<wire::Output> {
    if !crate::runtime::skript_version_is("2.15.4") {return None;}
    super::source::transfer(&request)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model()->Model {
        let rows=serde_json::from_str(include_str!("../../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/Types.json")).unwrap();
        Model::from_records(rows).unwrap()
    }
    fn request(op:&str,types:&[&str],nullable:bool)->wire::Request {
        wire::Request {subject:wire::Context {document_id:"fixture".into(),document_revision:1,node_id:1,syntax_context:0,
            source_digest:"source".into(),snapshot_id:"snapshot".into(),preparation_generation:1},operation:op.into(),
            parameters:if op==READ {serde_json::to_string(&Read {version:1,lookup:"local-scalar-empty".into(),requested_types:vec!["java.lang.Object".into()],no_refresh_types:vec!["java.lang.String".into()]}).unwrap()}
                else {serde_json::to_string(&Set {version:1,kind:"local-scalar-default-clone".into()}).unwrap()},
            input:bound(types.iter().map(|s|s.to_string()).collect(),nullable)}
    }
    #[test]
    fn immutable_and_empty_reads_and_sets_keep_exact_presence() {
        let m=model();
        for nullable in [false,true] {
            let read=evaluate(&m,&request(READ,&["java.lang.String"],nullable)).unwrap();
            assert_eq!(read.value.types,["java.lang.String"]);assert_eq!(read.value.maybe_unset,nullable);
            assert!(matches!(read.source,wire::SourceUpdate::Unchanged));
            let stored=evaluate(&m,&request(SET,&["java.lang.String"],nullable)).unwrap();
            assert_eq!(stored.value.types,["java.lang.String"]);assert_eq!(stored.value.maybe_unset,nullable);
        }
        for op in [READ,SET] {
            let empty=evaluate(&m,&request(op,&[],true)).unwrap();
            assert!(empty.value.types.is_empty()&&empty.value.maybe_unset&&empty.value.complete);
            assert!(matches!(empty.effects,wire::BindingEffects::Preserve));
        }
    }
    #[test]
    fn refresh_result_is_not_the_conditional_serialize_only_writeback() {
        let m=model();let read=evaluate(&m,&request(READ,&[REFRESH_DOMAIN],false)).unwrap();
        assert!(read.value.types.contains(&REFRESH_DOMAIN.into())&&read.value.maybe_unset);
        assert!(matches!(read.effects,wire::BindingEffects::Invalidate));
        assert!(matches!(read.source,wire::SourceUpdate::Unknown));
        let expected=m.stored_bounds(REFRESH_DOMAIN).unwrap().0;
        assert_eq!(read.source_writeback.unwrap().types.into_iter().collect::<BTreeSet<_>>(),expected);
    }
    #[test]
    fn unknown_writeback_does_not_erase_a_declared_read_result() {
        let mut m=model();
        for row in &mut m.records {row["typeRuntime"]["serializeAsState"]="unknown".into();}
        let read=evaluate(&m,&request(READ,&[REFRESH_DOMAIN],false)).unwrap();
        assert!(read.value.types.contains(&REFRESH_DOMAIN.into()));
        assert!(read.source_writeback.is_none()&&matches!(read.source,wire::SourceUpdate::Unknown));
        assert!(evaluate(&m,&wire::Request {input:read.value,..request(SET,&[],true)}).is_none());
    }
    #[test]
    fn unknown_user_type_uses_the_same_open_domain_rule_without_a_name_inventory() {
        let mut rows:Vec<Value>=serde_json::from_str(include_str!("../../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/Types.json")).unwrap();
        let mut extra=rows.iter().find(|r|r["originalClass"]=="java.lang.String").unwrap().clone();
        extra["originalClass"]="unseen.CustomValue".into();extra["typeParseOrder"]=(rows.len() as u64).into();
        extra["typeRuntime"]["classValue"]="unseen.CustomValue".into();extra["typeRuntime"]["classKind"]="Interface".into();
        extra["typeRuntime"]["ancestors"]=serde_json::json!(["java.lang.Object"]);rows.push(extra);
        let m=Model::from_records(rows).unwrap();
        let read=evaluate(&m,&request(READ,&["unseen.CustomValue"],false)).unwrap();
        assert!(read.value.types.contains(&"unseen.CustomValue".into()));
        assert!(read.value.types.contains(&REFRESH_DOMAIN.into()),"an open interface may overlap the standard refresh domain");
        let stored=evaluate(&m,&wire::Request {input:read.value,..request(SET,&[],true)}).unwrap();
        assert!(stored.value.complete&&stored.value.maybe_unset);
    }
    #[test]
    fn unsupported_lookup_partial_plural_unknown_and_overlarge_inputs_fail_closed() {
        let m=model();
        for case in ["partial","plural","unknown","default","old","too-many"] {
            let mut r=request(READ,&["java.lang.String"],false);
            match case {
                "partial"=>r.input.complete=false,"plural"=>r.input.multiple=true,
                "unknown"=>r.input.types=vec!["absent.Type".into()],
                "default"=>{let mut p:Value=serde_json::from_str(&r.parameters).unwrap();p["lookup"]="default-global-fallback".into();r.parameters=p.to_string();},
                "old"=>{let mut p:Value=serde_json::from_str(&r.parameters).unwrap();p["version"]=0.into();r.parameters=p.to_string();},
                "too-many"=>r.input.types=(0..65).map(|i|format!("type.{i}")).collect(),_=>unreachable!(),
            }
            assert!(evaluate(&m,&r).is_none(),"{case}");
        }
    }

    #[test]
    fn global_candidate_is_the_planned_value_not_current_shared_state() {
        let m=model();
        for nullable in [false,true] {
            let mut r=request(GLOBAL,&["java.lang.String"],nullable);
            r.parameters=serde_json::to_string(&Set {version:1,kind:"global-write-candidate-default-clone".into()}).unwrap();
            let out=evaluate(&m,&r).unwrap();
            assert_eq!(out.value.types,["java.lang.String"]);assert_eq!(out.value.maybe_unset,nullable);
            assert!(matches!(out.source,wire::SourceUpdate::Unknown));
            assert!(matches!(out.effects,wire::BindingEffects::Invalidate));assert!(out.source_writeback.is_none());
            r.parameters=serde_json::to_string(&Set {version:1,kind:"local-scalar-default-clone".into()}).unwrap();
            assert!(evaluate(&m,&r).is_none(),"global operation cannot masquerade as local storage");
        }
    }

    #[test]
    #[ignore = "requires an explicitly supplied saved SSG snapshot; set SKRIPT_TRANSFER_SNAPSHOT and run this test offline"]
    fn saved_actual_snapshot_can_compute_both_copy_domains_offline() {
        let root=std::env::var("SKRIPT_TRANSFER_SNAPSHOT").expect("set SKRIPT_TRANSFER_SNAPSHOT to the saved snapshot directory");
        let rows=serde_json::from_slice(&std::fs::read(std::path::Path::new(&root).join("Types.json")).unwrap()).unwrap();
        let m=Model::from_records(rows).unwrap();
        for types in [vec!["org.bukkit.Location",REFRESH_DOMAIN,"org.bukkit.inventory.ItemStack","org.joml.Quaternionf"],
            vec!["net.kyori.adventure.text.Component","org.bukkit.Location","org.bukkit.inventory.ItemStack","org.bukkit.util.Vector","org.joml.Quaternionf"]] {
            let read=evaluate(&m,&request(READ,&types,true)).unwrap();
            let stored=evaluate(&m,&wire::Request {input:read.value,..request(SET,&[],true)}).unwrap();
            assert!(stored.value.complete&&!stored.value.types.is_empty()&&stored.value.maybe_unset);
            println!("actual-copy-domain {}",serde_json::json!({"input":types,"stored":stored.value.types}));
        }
    }
}
