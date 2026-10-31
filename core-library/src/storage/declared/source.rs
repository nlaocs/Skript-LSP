//! Bounded immutable preparation; ordinary declaration contracts have a lifetime
//! separate from optional native-runtime certification.
use super::*;
use crate::nlaocs::skript_parser_addon::types::{MetadataEntry, ParseSummary, RegisteredExpressionPayload, SourcePreparationContext, SourcePreparationStatus, AddonError};
use crate::expressions::SemanticResolution;
use crate::storage::evidence::SourceIdentity;

#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Input {
    version:u32,
    source:SourceIdentity,
    definition:String,registration:String,class:String,pattern:u64,
    return_type:String,input_types:Vec<String>,
}
fn bounded_input(return_type:&str,types:&[String])->bool {
    !return_type.is_empty() && return_type.len()<=1024 && !types.is_empty() && types.len()<=64
        && types.iter().all(|t|!t.is_empty()&&t.len()<=1024&&t!="java.lang.Object")
        && types.iter().collect::<BTreeSet<_>>().len()==types.len()
}
impl Input {
    fn matches(&self,summary:&ParseSummary,source:&SourceIdentity)->bool {
        use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState};
        self.version==1 && bounded_input(&self.return_type,&self.input_types)
            && self.source==*source && summary.kind=="registered-expression"
            && summary.definition_id.as_ref()==Some(&self.definition)
            && summary.registration_id.as_ref()==Some(&self.registration)
            && summary.element_class.as_ref()==Some(&self.class)
            && summary.pattern_index==Some(self.pattern)
            && summary.return_type.as_ref()==Some(&self.return_type)
            && summary.possible_return_types==self.input_types
            && summary.possible_return_types_state==ExpressionPossibleReturnTypesState::Complete
            && summary.multiplicity==Some(DynamicMultiplicity::Single)
    }
}

pub(super) fn budget_label(declared_ready:bool)->&'static str {
    if declared_ready {"declared-storage-ready;native-target-unavailable:step-budget"}
    else {"declared-storage-unavailable;native-target-unavailable:step-budget"}
}

pub(crate) fn for_summary(summary:&ParseSummary)->Option<Inference> {
    #[cfg(target_arch="wasm32")]
    {
        let source=crate::storage::document::current_source().ok()?;
        guest::infer(&source,&input_types_at(summary,&source)?)
    }
    #[cfg(not(target_arch="wasm32"))]
    {let _=summary;None}
}
fn input_types_at(summary:&ParseSummary,source:&SourceIdentity)->Option<Vec<String>> {
    if summary.metadata.iter().any(|entry|entry.key==KEY) {
        // An explicit malformed/foreign/duplicate Core contract must not gain
        // a second chance from a different native witness.
        input_at(summary,source).map(|input|input.input_types)
    } else {
        crate::storage::native_inputs::declared_clone_bounds(summary)
    }
}
fn input_at(summary:&ParseSummary,source:&SourceIdentity)->Option<Input> {
    let mut entries=summary.metadata.iter().filter(|e|e.key==KEY);
    let e=entries.next()?;
    if entries.next().is_some() || e.owner_component_id.as_deref()!=Some(crate::COMPONENT_ID) {return None;}
    let input:Input=serde_json::from_str(&e.value).ok()?;
    input.matches(summary,source).then_some(input)
}

#[cfg(target_arch="wasm32")]
mod guest {
    use super::*;
    use std::sync::{Arc,Mutex,LazyLock,RwLock};
    use crate::nlaocs::skript_parser_addon::catalog_data;
    struct Prepared {model:Model,generation:u64,methods:RwLock<BTreeMap<String,bool>>}
    static MODELS:LazyLock<RwLock<BTreeMap<String,Arc<Prepared>>>>=LazyLock::new(||RwLock::new(BTreeMap::new()));
    struct Job {source:SourceIdentity,generation:u64,next:u32,phase:usize,offset:u64,types:Vec<Value>,native_start:Option<u32>,declared_ready:bool}
    static JOB:Mutex<Option<Job>>=Mutex::new(None);
    fn key(s:&SourceIdentity)->String {format!("{}:{}",s.source_digest,s.snapshot_id)}
    fn current()->Option<SourceIdentity> {crate::storage::document::current_source().ok()}
    fn supported()->bool {crate::runtime::current().as_ref().and_then(|p|p.skript_version.as_deref()).is_some_and(supports)}
    pub(super) fn infer(source:&SourceIdentity,input_types:&[String])->Option<Inference> {
        if !supported() || current().as_ref()!=Some(source) {return None;}
        let prepared=MODELS.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key(source)).cloned()?;
        let result=prepared.model.infer(input_types)?;
        (current().as_ref()==Some(source)).then_some(result)
    }
    pub(super) fn transfer(request:&crate::exports::nlaocs::skript_parser_addon::semantic_transfer::Request)
        ->Option<crate::exports::nlaocs::skript_parser_addon::semantic_transfer::Output> {
        // No StateStore import is needed in the data-only query. Source and
        // preparation generation are supplied by the authenticated host session.
        let source=SourceIdentity {source_digest:request.subject.source_digest.clone(),snapshot_id:request.subject.snapshot_id.clone()};
        let prepared=MODELS.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key(&source)).cloned()?;
        if prepared.generation!=request.subject.preparation_generation {return None;}
        super::super::transfer::evaluate(&prepared.model,request)
    }
    pub(super) fn prepare(mut context:SourcePreparationContext)->Result<SourcePreparationStatus,AddonError> {
        if context.generation==0 {
            *JOB.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=None;
            return Ok(SourcePreparationStatus::Unavailable("declared-generation-zero".into()));
        }
        if !supported() || context.snapshot_id.is_none() && context.source_digest.is_none() {
            return crate::storage::preparation::native_step(context);
        }
        let Some(source)=current().filter(|s|context.snapshot_id.as_ref()==Some(&s.snapshot_id)&&context.source_digest.as_ref()==Some(&s.source_digest)) else {
            *JOB.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=None;
            return Ok(SourcePreparationStatus::Unavailable("declared-source-mismatch".into()));
        };
        let mut slot=JOB.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if context.step==0 {
            MODELS.write().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&key(&source));
            *slot=Some(Job {source:source.clone(),generation:context.generation,next:0,phase:0,offset:0,types:Vec::new(),native_start:None,declared_ready:false});
        }
        let Some(job)=slot.as_mut().filter(|j|j.generation==context.generation&&j.source==source&&j.next==context.step) else {
            *slot=None; return Ok(SourcePreparationStatus::Unavailable("declared-step-mismatch".into()));
        };
        job.next+=1;
        if let Some(start)=job.native_start {
            let last=context.step==63;
            let declared_ready=job.declared_ready;
            context.step-=start; drop(slot);
            let outcome=crate::storage::preparation::native_step(context)?;
            if last && matches!(outcome,SourcePreparationStatus::Pending(_)) {
                crate::storage::preparation::clear();
                return Ok(SourcePreparationStatus::Ready(budget_label(declared_ready).into()));
            }
            return Ok(outcome);
        }
        let result=(||->Option<SourcePreparationStatus>{
            if job.phase==0 {
                let (name,fields,limit,count)=("Types.json",vec!["originalClass","typeParseOrder","typeRuntime","serializeAs"],1024usize,64u32);
                let fields=fields.into_iter().map(str::to_owned).collect::<Vec<_>>();
                let page=catalog_data::read_json_records(&source.source_digest,&source.snapshot_id,name,&[],&fields,job.offset,count,32768).ok()??;
                if page.total_records>limit as u64 || page.items.len()>count as usize {return None;}
                let end=job.offset.checked_add(page.items.len() as u64)?;
                if end>page.total_records || page.next_offset!=if end<page.total_records {Some(end)} else {None}
                    || end<page.total_records&&page.items.is_empty() {return None;}
                for (i,item) in page.items.into_iter().enumerate() {
                    if item.index!=job.offset+i as u64 || item.json.len()>32768 {return None;}
                    job.types.push(serde_json::from_slice(&item.json).ok()?);
                }
                if let Some(next)=page.next_offset {job.offset=next;} else {job.phase+=1;job.offset=0;}
                if current().as_ref()!=Some(&source) {return None;}
                return Some(SourcePreparationStatus::Pending(format!("declared:{name}")));
            }
            let model=Model::from_records(std::mem::take(&mut job.types))?;
            if current().as_ref()!=Some(&source) {return None;}
            let mut models=MODELS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            if models.len()>=16 {models.clear();}
            models.insert(key(&source),Arc::new(Prepared {model,generation:context.generation,methods:RwLock::new(BTreeMap::new())}));
            job.declared_ready=true;
            Some(SourcePreparationStatus::Pending("declared-storage-ready".into()))
        })();
        // A failed static-contract preparation does not authorize a result and
        // does not masquerade as a native certificate. The independent prior
        // native path still receives its own step zero and bounded budget.
        if result.is_none()||job.phase==1&&job.types.is_empty() {
            job.native_start=Some(context.step+1);
        }
        Ok(result.unwrap_or_else(||SourcePreparationStatus::Pending("declared-storage-unavailable".into())))
    }
    pub(super) fn annotate(payload:&RegisteredExpressionPayload,resolution:&mut SemanticResolution,delegate:Option<DelegatedInput>) {
        use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState};
        if !supported() {return;}
        let SemanticResolution::Resolved {return_type,possible_return_types,possible_return_types_state,multiplicity,metadata}=resolution else {return;};
        if *multiplicity!=DynamicMultiplicity::Single || *possible_return_types_state!=ExpressionPossibleReturnTypesState::Complete
            || !bounded_input(return_type,possible_return_types) || metadata.iter().any(|e|e.key==KEY) {return;}
        let Some(source)=current() else{return;};
        let Some(prepared)=MODELS.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key(&source)).cloned() else{return;};
        let method_key=serde_json::json!([payload.registration_id,payload.definition_id,payload.element_class,payload.pattern_index,payload.pattern,
            delegate.map(|d|[d.adapter,d.forwarder,d.receiver])]).to_string();
        if method_key.len()>4096 {return;}
        let cached=prepared.methods.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&method_key).copied();
        let valid=cached.unwrap_or_else(||{
            let records=crate::catalog::registration_records(&payload.registration_id).ok();
            let valid=records.as_ref().is_some_and(|rows|{
                let rows=rows.iter().filter(|r|r.document=="Expressions.json").collect::<Vec<_>>();
                rows.len()==1 && serde_json::from_slice::<Value>(&rows[0].bytes).ok().is_some_and(|r|
                    r["kind"]=="expression" && r["elementClass"]==payload.element_class && r["definitionId"]==payload.definition_id
                    && r["registrationId"]==payload.registration_id && r["patterns"].get(payload.pattern_index as usize).and_then(Value::as_str)==Some(payload.pattern.as_str())
                    && delegate.is_none_or(|_|r["addon"]["name"]=="Skript" && payload.definition_id.starts_with("expression:skript:")))
            }) && catalog_data::class_lineage(&payload.element_class).ok().flatten().is_some_and(|closure| {
                let declares=|c:&str|crate::catalog::declared_method_exists(c,"beforeChange",&[EXPRESSION,"java.lang.Object[]"],Some("java.lang.Object[]")).ok().flatten();
                if let Some(d)=delegate {
                    forwarding_hook(&closure,d.forwarder,declares)
                        && catalog_data::class_lineage(d.receiver).ok().flatten().is_some_and(|receiver|default_hook(&receiver,declares))
                } else {default_hook(&closure,declares)}
            });
            // A failed/unknown host query is not a permanent negative fact.
            if valid && current().as_ref()==Some(&source) {
                let mut cache=prepared.methods.write().unwrap_or_else(std::sync::PoisonError::into_inner);
                if cache.len()>=1024 {cache.clear();} cache.insert(method_key,valid);
            }
            valid
        });
        if !valid || current().as_ref()!=Some(&source) {return;}
        // Only the default beforeChange input contract is recorded here. Storage
        // transfer is evaluated at the consuming SET, after any stronger native
        // certificate, so expression resolution does not repeat that work.
        let input=Input {version:1,source,definition:payload.definition_id.clone(),registration:payload.registration_id.clone(),
            class:payload.element_class.clone(),pattern:payload.pattern_index,return_type:return_type.clone(),input_types:possible_return_types.clone()};
        metadata.push(MetadataEntry {key:KEY.into(),value:serde_json::to_string(&input).expect("input serializes"),owner_component_id:None});
    }
}
pub(crate) fn prepare_step(context:SourcePreparationContext)->Result<SourcePreparationStatus,AddonError> {
    #[cfg(target_arch="wasm32")] {guest::prepare(context)}
    #[cfg(not(target_arch="wasm32"))] {crate::storage::preparation::native_step(context)}
}
pub(super) fn transfer(request:&crate::exports::nlaocs::skript_parser_addon::semantic_transfer::Request)
    ->Option<crate::exports::nlaocs::skript_parser_addon::semantic_transfer::Output> {
    #[cfg(target_arch="wasm32")] {guest::transfer(request)}
    #[cfg(not(target_arch="wasm32"))] {let _=request;None}
}
pub(crate) fn annotate(payload:&RegisteredExpressionPayload,resolution:&mut SemanticResolution,delegate:Option<DelegatedInput>) {
    #[cfg(target_arch="wasm32")] guest::annotate(payload,resolution,delegate);
    #[cfg(not(target_arch="wasm32"))] let _=(payload,resolution,delegate);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState};
    #[test]
    fn a_transform_cannot_reuse_an_input_for_a_different_effective_summary_or_source() {
        let source=SourceIdentity {snapshot_id:"snapshot".into(),source_digest:"digest".into()};
        let input=Input {version:1,source:source.clone(),definition:"definition".into(),registration:"registration".into(),class:"owned.Getter".into(),
            pattern:0,return_type:"owned.Value".into(),input_types:vec!["owned.Value".into()]};
        let summary=ParseSummary {kind:"registered-expression".into(),definition_id:Some("definition".into()),registration_id:Some("registration".into()),
            element_class:Some("owned.Getter".into()),pattern_index:Some(0),return_type:Some("owned.Value".into()),possible_return_types:vec!["owned.Value".into()],
            possible_return_types_state:ExpressionPossibleReturnTypesState::Complete,multiplicity:Some(DynamicMultiplicity::Single),default_expression:None,
            public_data:Vec::new(),metadata:vec![MetadataEntry {key:KEY.into(),value:serde_json::to_string(&input).unwrap(),owner_component_id:Some(crate::COMPONENT_ID.into())}]};
        assert_eq!(input_at(&summary,&source).unwrap().input_types,["owned.Value"]);
        let mut changed=summary.clone();changed.possible_return_types=vec!["owned.Other".into()];assert!(input_at(&changed,&source).is_none());
        let mut changed=summary.clone();changed.return_type=Some("owned.Other".into());assert!(input_at(&changed,&source).is_none());
        let mut changed=summary.clone();changed.multiplicity=Some(DynamicMultiplicity::Multiple);assert!(input_at(&changed,&source).is_none());
        let mut changed=summary.clone();changed.possible_return_types_state=ExpressionPossibleReturnTypesState::Partial;assert!(input_at(&changed,&source).is_none());
        let mut changed=summary.clone();changed.metadata[0].owner_component_id=Some("foreign".into());assert!(input_at(&changed,&source).is_none());
        let mut changed=summary.clone();changed.metadata.push(changed.metadata[0].clone());assert!(input_at(&changed,&source).is_none());
        let mut changed_source=source.clone();changed_source.source_digest="other".into();assert!(input_at(&summary,&changed_source).is_none());
        changed_source=source.clone();changed_source.snapshot_id="other".into();assert!(input_at(&summary,&changed_source).is_none());
        let mut changed=summary.clone();let mut json=serde_json::to_value(&input).unwrap();json["result"]=serde_json::json!({"complete":true});
        changed.metadata[0].value=json.to_string();assert!(input_at(&changed,&source).is_none());
        let mut changed=summary.clone();let mut json=serde_json::to_value(&input).unwrap();json["version"]=2.into();
        changed.metadata[0].value=json.to_string();assert!(input_at(&changed,&source).is_none());
    }
    #[test]
    fn default_hook_input_bounds_do_not_authorize_object_partial_or_unbounded_types() {
        assert!(bounded_input("owned.Value",&["owned.Value".into()]));
        assert!(!bounded_input("owned.Value",&[]));
        assert!(!bounded_input("owned.Value",&["java.lang.Object".into()]));
        assert!(!bounded_input("owned.Value",&["owned.Value".into(),"owned.Value".into()]));
        assert!(!bounded_input("owned.Value",&["x".repeat(1025)]));
        assert!(!bounded_input(&"x".repeat(1025),&["owned.Value".into()]));
        assert!(!bounded_input("owned.Value",&(0..65).map(|i|format!("owned.Type{i}")).collect::<Vec<_>>()));
    }
    #[test]
    fn explicit_core_input_contract_has_priority_and_cannot_fall_back_to_a_native_callback() {
        let source=SourceIdentity {snapshot_id:"snapshot".into(),source_digest:"digest".into()};
        let input=Input {version:1,source:source.clone(),definition:"definition".into(),registration:"registration".into(),class:"owned.Getter".into(),
            pattern:0,return_type:"owned.Value".into(),input_types:vec!["owned.Value".into()]};
        let mut summary=ParseSummary {kind:"registered-expression".into(),definition_id:Some("definition".into()),registration_id:Some("registration".into()),
            element_class:Some("owned.Getter".into()),pattern_index:Some(0),return_type:Some("owned.Value".into()),possible_return_types:vec!["owned.Value".into()],
            possible_return_types_state:ExpressionPossibleReturnTypesState::Complete,multiplicity:Some(DynamicMultiplicity::Single),default_expression:None,
            public_data:vec![],metadata:vec![MetadataEntry {key:KEY.into(),value:serde_json::to_string(&input).unwrap(),owner_component_id:Some(crate::COMPONENT_ID.into())},
                MetadataEntry {key:crate::storage::native_inputs::INPUT.into(),owner_component_id:None,value:serde_json::json!({"version":1,"receipt":7,
                    "subject":"a".repeat(64),"issuer":"foreign.provider","valueDomain":{"version":1,"classes":[],"complete":false,"maybeUnset":false,
                    "beforeChange":"default-clone","evidence":"audited callback only"}}).to_string()}]};
        assert_eq!(input_types_at(&summary,&source).unwrap(),["owned.Value"]);
        let mut no_core=summary.clone();no_core.metadata.remove(0);
        assert_eq!(input_types_at(&no_core,&source).unwrap(),["owned.Value"]);
        for case in 0..6 {
            let mut changed=summary.clone();let mut current=source.clone();
            match case {
                0=>changed.metadata[0].owner_component_id=Some("foreign".into()),
                1=>changed.metadata[0].value="{".into(),
                2=>changed.metadata.push(changed.metadata[0].clone()),
                3=>current.source_digest="other".into(),
                4=>current.snapshot_id="other".into(),
                _=>{let mut old=serde_json::to_value(&input).unwrap();old["version"]=0.into();changed.metadata[0].value=old.to_string();},
            }
            assert!(input_types_at(&changed,&current).is_none(),"explicit invalid input {case}");
        }
        summary.metadata[1].value="invalid native seal".into();
        assert_eq!(input_types_at(&summary,&source).unwrap(),["owned.Value"],"the accepted exact Core source remains the primary path");
    }
}
