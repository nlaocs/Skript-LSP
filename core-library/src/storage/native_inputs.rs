//! Native-sealed foreign value inputs. No guest metadata ownership is promoted
//! to Core ownership, and evaluation effects are not certified by this contract.
use super::*;
pub(crate) const INPUT: &str = "parser.native-semantic-input.v1";
pub(crate) const COMPOSE: &str = "parser.semantic.compose.v1";
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Domain {
    version:u32, classes:Vec<String>, complete:bool, maybe_unset:bool,
    before_change:String, evidence:String,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Input {
    version:u32, receipt:u64, subject:String, issuer:String, value_domain:Option<Domain>,
}
fn input(summary:&ParseSummary)->Option<Input> {
    let mut entries=summary.metadata.iter().filter(|e|e.key==INPUT);
    let entry=entries.next()?;
    if entries.next().is_some() || entry.owner_component_id.is_some() || entry.value.len()>16*1024 {return None;}
    let input:Input=serde_json::from_str(&entry.value).ok()?;
    (input.version==1 && input.receipt>0 && !input.issuer.is_empty() && input.issuer.len()<=256
        && input.subject.len()==64 && input.subject.bytes().all(|b|b.is_ascii_hexdigit())).then_some(input)
}
/// An authenticated callback contract does not make its declared public bounds
/// an actual runtime-class domain. The ordinary storage model handles them.
pub(crate) fn declared_clone_bounds(summary:&ParseSummary)->Option<Vec<String>> {
    default_clone_callback(summary)?;
    declared_bounds(summary)
}
fn default_clone_callback(summary:&ParseSummary)->Option<()> {
    let domain=input(summary)?.value_domain?;
    (domain.version==1 && domain.classes.len()<=16 && (!domain.complete || !domain.classes.is_empty())
        && domain.classes.iter().all(|c|!c.is_empty()&&c.len()<=1024&&c!="java.lang.Object")
        && domain.classes.iter().collect::<std::collections::BTreeSet<_>>().len()==domain.classes.len()
        && domain.before_change=="default-clone" && !domain.evidence.is_empty() && domain.evidence.len()<=1024).then_some(())
}
fn declared_bounds(summary:&ParseSummary)->Option<Vec<String>> {
    use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState};
    let return_type=summary.return_type.as_deref()?;
    let classes=&summary.possible_return_types;
    (summary.multiplicity==Some(DynamicMultiplicity::Single)
        && summary.possible_return_types_state==ExpressionPossibleReturnTypesState::Complete
        && !return_type.is_empty() && return_type.len()<=1024
        && !classes.is_empty() && classes.len()<=64
        && classes.iter().all(|c|!c.is_empty()&&c.len()<=1024&&c!="java.lang.Object")
        && classes.iter().collect::<std::collections::BTreeSet<_>>().len()==classes.len()).then(||classes.clone())
}
pub(crate) fn clone_classes(summary:&ParseSummary)->Option<Vec<String>> {
    if summary.multiplicity!=Some(crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity::Single) {return None;}
    let domain=input(summary)?.value_domain?;
    (domain.version==1 && domain.complete && !domain.classes.is_empty() && domain.classes.len()<=16
        && domain.classes.iter().all(|c|!c.is_empty()&&c.len()<=1024&&c!="java.lang.Object")
        && domain.classes.iter().collect::<std::collections::BTreeSet<_>>().len()==domain.classes.len()
        && domain.before_change=="default-clone" && !domain.evidence.is_empty() && domain.evidence.len()<=1024).then_some(domain.classes)
}
fn stored_with_classes(summary:&ParseSummary,identity:&[String])->Option<ResultType> {
    let classes=clone_classes(summary)?;
    if !classes.iter().all(|class|identity.contains(class)) {return None;}
    let domain=input(summary)?.value_domain?;
    Some(ResultType {version:1,scope:"local-scalar".into(),types:classes,complete:true,multiple:false,
        maybe_unset:domain.maybe_unset,preserves_local_bindings:true})
}
pub(crate) fn stored(summary:&ParseSummary)->Option<ResultType> {
    stored_with_classes(summary,&crate::catalog::identity_delta_classes())
}
pub(crate) fn compose(target:&ParseSummary,value:&ParseSummary,target_capture:u64,value_capture:u64,context_preserving:bool)->Option<String> {
    let target_input=input(target)?;
    if target_input.issuer!=crate::COMPONENT_ID || input(value)?.value_domain.is_none() {return None;}
    let mut keys=vec!["skript.semantic.variable-change","skript.semantic.operand-evaluation"];
    if context_preserving {keys.push(crate::public_data::EVALUATION_CONTEXT_METADATA_KEY);}
    Some(serde_json::json!({"version":1,"metadataKeys":keys,"inputs":[
        {"capture":target_capture,"publicSchemas":[crate::public_data::VARIABLE_IDENTITY_SCHEMA_ID]},
        {"capture":value_capture,"publicSchemas":[]}
    ]}).to_string())
}
/// A deferred flow transfer does not assert the delta's value domain. It still
/// binds both exact current input subjects and the variable target identity.
pub(crate) fn compose_transfer(target:&ParseSummary,value:&ParseSummary,target_capture:u64,value_capture:u64,context_preserving:bool)->Option<String> {
    if input(target)?.issuer!=crate::COMPONENT_ID || input(value)?.issuer!=crate::COMPONENT_ID {return None;}
    let mut keys=vec!["skript.semantic.variable-change","skript.semantic.operand-evaluation","parser.semantic-transfer.v1"];
    if context_preserving {keys.push(crate::public_data::EVALUATION_CONTEXT_METADATA_KEY);}
    Some(serde_json::json!({"version":1,"metadataKeys":keys,"inputs":[
        {"capture":target_capture,"publicSchemas":[crate::public_data::VARIABLE_IDENTITY_SCHEMA_ID]},
        {"capture":value_capture,"publicSchemas":[]}
    ]}).to_string())
}

/// Seal intrinsic effects against one current parsed operand. Evaluation of
/// that operand is not certified by this composition.
pub(crate) fn compose_effect_input(value:&ParseSummary,capture:u64,keys:&[&str])->Option<String> {
    input(value)?;
    Some(serde_json::json!({"version":1,"metadataKeys":keys,"inputs":[{"capture":capture,"publicSchemas":[]}]}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn summary()->ParseSummary {
        use crate::nlaocs::skript_parser_addon::types::*;
        ParseSummary {kind:"registered-expression".into(),definition_id:Some("owned.call".into()),registration_id:Some("owned.call.1".into()),
            element_class:Some("owned.Call".into()),pattern_index:Some(0),return_type:Some("java.lang.Object".into()),possible_return_types:vec![],
            possible_return_types_state:ExpressionPossibleReturnTypesState::Unresolved,multiplicity:Some(DynamicMultiplicity::Single),default_expression:None,
            public_data:vec![],metadata:vec![MetadataEntry {key:INPUT.into(),owner_component_id:None,value:serde_json::json!({"version":1,"receipt":7,"subject":"a".repeat(64),"issuer":"owned.provider",
                "valueDomain":{"version":1,"classes":["java.lang.String"],"complete":true,"maybeUnset":true,"beforeChange":"default-clone","evidence":"owned source model"}}).to_string()}]}
    }
    fn callback_only()->ParseSummary {
        use crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState;
        let mut s=summary();
        s.return_type=Some("custom.Child".into());
        s.possible_return_types=vec!["custom.Child".into()];
        s.possible_return_types_state=ExpressionPossibleReturnTypesState::Complete;
        let mut value:serde_json::Value=serde_json::from_str(&s.metadata[0].value).unwrap();
        value["valueDomain"]["classes"]=serde_json::json!([]);
        value["valueDomain"]["complete"]=false.into();
        value["valueDomain"]["maybeUnset"]=false.into();
        s.metadata[0].value=value.to_string();
        s
    }
    #[test]
    fn callback_only_contract_does_not_promote_declared_bounds_to_runtime_classes() {
        let s=callback_only();
        assert!(default_clone_callback(&s).is_some());
        assert_eq!(declared_clone_bounds(&s).unwrap(),["custom.Child"]);
        assert!(clone_classes(&s).is_none());
        assert!(stored_with_classes(&s,&["custom.Child".into()]).is_none());
        assert!(s.metadata.iter().all(|e|e.key!=crate::public_data::LOCAL_EFFECTS_METADATA_KEY));
        let mut stronger=s.clone();
        let mut value:serde_json::Value=serde_json::from_str(&stronger.metadata[0].value).unwrap();
        value["valueDomain"]["classes"]=serde_json::json!(["java.lang.String"]);
        value["valueDomain"]["complete"]=true.into();
        stronger.metadata[0].value=value.to_string();
        assert_eq!(clone_classes(&stronger).unwrap(),["java.lang.String"]);
        assert_eq!(declared_clone_bounds(&stronger).unwrap(),["custom.Child"],"the public upper bound is independent of the runtime domain");
    }
    #[test]
    fn callback_only_contract_rejects_malformed_duplicate_foreign_or_old_native_seals() {
        for field in ["version","receipt","subject","issuer","owner","copied","malformed","domain-version","beforeChange","evidence","evidence-size","classes","class-count","complete","no-domain","unknown-field","duplicate-field"] {
            let mut s=callback_only();
            let mut value:serde_json::Value=serde_json::from_str(&s.metadata[0].value).unwrap();
            match field {
                "version"=>value[field]=0.into(),
                "receipt"=>value[field]=0.into(),
                "subject"=>value[field]="old-or-invalid".into(),
                "issuer"=>value[field]="".into(),
                "owner"=>s.metadata[0].owner_component_id=Some("foreign".into()),
                "copied"=>s.metadata.push(s.metadata[0].clone()),
                "domain-version"=>value["valueDomain"]["version"]=0.into(),
                "beforeChange"=>value["valueDomain"][field]="unknown".into(),
                "evidence"=>value["valueDomain"][field]="".into(),
                "evidence-size"=>value["valueDomain"]["evidence"]="x".repeat(1025).into(),
                "classes"=>value["valueDomain"][field]=serde_json::json!(["java.lang.Object"]),
                "class-count"=>value["valueDomain"]["classes"]=serde_json::json!((0..17).map(|i|format!("custom.Class{i}")).collect::<Vec<_>>()),
                "complete"=>value["valueDomain"][field]=true.into(),
                "no-domain"=>value["valueDomain"]=serde_json::Value::Null,
                "unknown-field"=>value["valueDomain"]["purity"]="preserve".into(),
                _=>{},
            }
            s.metadata[0].value=match field {
                "malformed"=>"{".into(),
                "duplicate-field"=>format!("{{\"version\":1,{}",&value.to_string()[1..]),
                _=>value.to_string(),
            };
            assert!(declared_clone_bounds(&s).is_none(),"{field}");
        }
    }
    #[test]
    fn callback_only_public_bounds_require_single_complete_bounded_non_object_types() {
        use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState};
        for field in ["partial","unresolved","multiple","unknown-cardinality","object","empty","duplicate","empty-class","class-size","class-count","return-empty","return-size","no-return"] {
            let mut s=callback_only();
            match field {
                "partial"=>s.possible_return_types_state=ExpressionPossibleReturnTypesState::Partial,
                "unresolved"=>s.possible_return_types_state=ExpressionPossibleReturnTypesState::Unresolved,
                "multiple"=>s.multiplicity=Some(DynamicMultiplicity::Multiple),
                "unknown-cardinality"=>s.multiplicity=None,
                "object"=>s.possible_return_types=vec!["java.lang.Object".into()],
                "empty"=>s.possible_return_types.clear(),
                "duplicate"=>s.possible_return_types.push(s.possible_return_types[0].clone()),
                "empty-class"=>s.possible_return_types=vec!["".into()],
                "class-size"=>s.possible_return_types=vec!["x".repeat(1025)],
                "class-count"=>s.possible_return_types=(0..65).map(|i|format!("custom.Class{i}")).collect(),
                "return-empty"=>s.return_type=Some("".into()),
                "return-size"=>s.return_type=Some("x".repeat(1025)),
                _=>s.return_type=None,
            }
            assert!(default_clone_callback(&s).is_some(),"callback and public bounds are independent: {field}");
            assert!(declared_clone_bounds(&s).is_none(),"{field}");
        }
    }
    #[test]
    fn callback_only_composition_still_requires_the_core_target_and_current_native_inputs() {
        let value=callback_only();let mut target=summary();
        let mut seal:serde_json::Value=serde_json::from_str(&target.metadata[0].value).unwrap();
        seal["issuer"]=crate::COMPONENT_ID.into();target.metadata[0].value=seal.to_string();
        assert!(compose(&target,&value,0,1,false).is_some());
        assert!(compose_transfer(&target,&value,0,1,false).is_none(),"foreign callback contracts do not authorize Core flow operations");
        assert!(compose(&value,&value,0,1,false).is_none());
        let mut duplicated=value.clone();duplicated.metadata.push(duplicated.metadata[0].clone());
        assert!(compose(&target,&duplicated,0,1,false).is_none());
    }
    #[test]
    fn native_domain_is_separate_from_advertised_object_and_evaluation_effects() {
        let s=summary();let r=stored_with_classes(&s,&["java.lang.String".into()]).unwrap();
        assert_eq!(r.types,["java.lang.String"]);assert!(r.maybe_unset);assert!(r.preserves_local_bindings);
        assert!(s.metadata.iter().all(|e|e.key!=crate::public_data::LOCAL_EFFECTS_METADATA_KEY));
        assert!(stored_with_classes(&s,&[]).is_none(),"actual ClassInfo storage proof is still required");
    }
    #[test]
    fn domain_storage_rejects_unknown_callback_partial_plural_or_guest_owned_seal() {
        for field in ["beforeChange","complete","classes","version","duplicate","owner","multiple","copied"] {
            let mut s=summary();let mut value:serde_json::Value=serde_json::from_str(&s.metadata[0].value).unwrap();
            match field {"beforeChange"=>value["valueDomain"][field]="unknown".into(),"complete"=>value["valueDomain"][field]=false.into(),
                "classes"=>value["valueDomain"][field]=serde_json::json!(["org.bukkit.entity.Player"]),"version"=>value[field]=2.into(),
                "duplicate"=>value["valueDomain"]["classes"]=serde_json::json!(["java.lang.String","java.lang.String"]),
                "owner"=>s.metadata[0].owner_component_id=Some("provider".into()),
                "multiple"=>s.multiplicity=Some(crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity::Multiple),
                _=>s.metadata.push(s.metadata[0].clone())}
            s.metadata[0].value=value.to_string();assert!(stored_with_classes(&s,&["java.lang.String".into()]).is_none(),"{field}");
        }
    }
}
