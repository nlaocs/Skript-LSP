use super::*;
use skript_parser::{
    EffectSemanticDecision, EffectSemanticRequest, PortableStructureBody, StructureBodyPlan,
    StructureBodyReplay, StructureBodyReuseRequest, StructureBodyWork, StructureHookDecision,
};

#[derive(Default)]
struct ReuseEnvironment {
    cached: Option<StructureBodyReplay>,
    pending: Option<StructureBodyReplay>,
    preparations: Vec<skript_parser::ReusedSectionPreparation>,
    notices: Vec<String>,
    before: Option<StructureBodyWork>,
    prepares: usize,
    effects: usize,
    exits: usize,
    adoptions: Vec<bool>,
    cancel_at: Option<&'static str>,
    cancelled: bool,
    bad_layout: bool,
    reject_exit: bool,
    measure_admission: bool,
    admission_counts: std::cell::RefCell<BTreeMap<&'static str,usize>>,
    enabled_calls: std::cell::Cell<usize>,
    disabled: bool,
    forced_mode: Option<skript_parser::StructureBodyMode>,
    semantic_order: Vec<&'static str>,
}

impl PatternMatchEnvironment for ReuseEnvironment {
    fn measurement_scope(&self, name: &'static str) -> skript_parser::ParserMeasurementScope {
        if self.measure_admission && name.starts_with("body_admission.") {
            *self.admission_counts.borrow_mut().entry(name).or_default() += 1;
        }
        skript_parser::ParserMeasurementScope::default()
    }
    fn cancellation_requested(&mut self) -> bool { self.cancelled }
    fn resolve_type(&mut self, _: TypeExpressionRequest<'_>) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }
    fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for ReuseEnvironment {
    fn state_revision(&self) -> Result<u64, String> { Ok(0) }
    fn parse_expression_leaf(&mut self, _: ExpressionLeafRequest<'_>) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }
    fn enter_structure(&mut self, request: StructureHookRequest<'_>) -> Result<StructureHookDecision, String> {
        self.semantic_order.push("enter");
        Ok(StructureHookDecision::Accept {
            context: request.context.clone(), body_mode: self.forced_mode.unwrap_or(skript_parser::StructureBodyMode::Trigger),
            entry_body_modes: if self.bad_layout { BTreeMap::from([(vec!["missing".into()],skript_parser::SectionBodyMode::Trigger)]) } else { BTreeMap::new() },
            metadata: request.candidate.metadata.clone(),
        })
    }
    fn resolve_effect_candidate(&mut self, _: EffectSemanticRequest<'_>) -> Result<EffectSemanticDecision, String> {
        self.effects += 1;
        self.semantic_order.push("effect");
        Ok(EffectSemanticDecision::UseCandidate)
    }
    fn exit_structure(&mut self, _: StructureHookRequest<'_>) -> Result<StructureExitDecision, String> {
        self.exits += 1;
        self.semantic_order.push("exit");
        self.cancelled |= self.cancel_at == Some("exit");
        if self.reject_exit {Ok(StructureExitDecision::Reject {reason:"test parent rejection".into(),diagnostics:Vec::new()})}
        else {Ok(StructureExitDecision::Accept)}
    }
    fn enter_section_children(&mut self, request:skript_parser::SectionChildrenRequest<'_>)->Result<skript_parser::SectionChildrenDecision,String> {
        let mut metadata=request.metadata.clone();metadata.insert("phase".into(),"enter".into());
        Ok(skript_parser::SectionChildrenDecision::Accept {context:request.context.clone(),body_mode:skript_parser::SectionBodyMode::Trigger,metadata})
    }
    fn exit_section_children(&mut self, request:skript_parser::SectionChildrenRequest<'_>)->Result<skript_parser::SectionExitDecision,String> {
        let mut metadata=request.metadata.clone();metadata.insert("phase".into(),"exit".into());
        Ok(skript_parser::SectionExitDecision::Accept {context:request.context.clone(),metadata})
    }
    fn structure_preparation(&mut self, event:skript_parser::StructurePreparationEvent<'_>) {
        use skript_parser::StructurePreparationEvent;
        match event {
            StructurePreparationEvent::SectionHeader {candidate,attempt_id,parent_attempt_id,..}=>{
                assert_eq!(candidate.metadata.get("phase").map(String::as_str),Some("enter"));
                self.notices.push(format!("section:{attempt_id}:{parent_attempt_id:?}"));
                let (candidate,_)=PortableStructureBody::freeze_preparation(candidate,&mut |_,_|false).unwrap();
                self.preparations.push(skript_parser::ReusedSectionPreparation {candidate,attempt_id,parent_attempt_id});
                self.cancelled|=self.cancel_at==Some("publication");
            },
            StructurePreparationEvent::Invalidated {attempt_id,..}=>self.notices.push(format!("invalid:{attempt_id}")),
            _=>{},
        }
    }
    fn structure_body_reuse_enabled(&self) -> bool {
        self.enabled_calls.set(self.enabled_calls.get()+1);
        !self.disabled
    }
    fn prepare_structure_body(&mut self, request: StructureBodyReuseRequest<'_>) -> Result<StructureBodyPlan, String> {
        self.prepares += 1;
        self.pending=None;
        self.preparations.clear();
        self.before=Some(request.work);
        self.cancelled |= self.cancel_at == Some("prepare");
        Ok(self.cached.clone().map_or(StructureBodyPlan::Observe,StructureBodyPlan::Reuse))
    }
    fn accept_structure_body_replay(&mut self, _: StructureBodyReuseRequest<'_>) -> Result<bool,String> {
        self.cancelled |= self.cancel_at == Some("admit");
        Ok(true)
    }
    fn finish_structure_body(&mut self, body: &mut StructureBody, context: &ExpressionParseContext, work: StructureBodyWork) -> Result<bool,String> {
        self.pending=Some(StructureBodyReplay {
            before:self.before.unwrap(), body:PortableStructureBody::freeze(body,&mut |_,_|false).unwrap(),
            post_context:context.clone(),work,preparations:self.preparations.clone(),
        });
        self.cancelled |= self.cancel_at == Some("finish");
        Ok(true)
    }
    fn adopt_structure_body(&mut self, accepted:bool) -> Result<(),String> {
        self.adoptions.push(accepted);
        self.semantic_order.push("adopt");
        self.cancelled |= self.cancel_at == Some("adopt");
        if accepted && !self.cancelled {self.cached=self.pending.take();}
        else {self.pending=None;}
        Ok(())
    }
    fn finish_semantic_candidate(&mut self, _:bool) -> Result<(),String> {
        self.cancelled |= self.prepares>1 && self.cancel_at==Some("semantic_finish");
        Ok(())
    }
}

fn fixture() -> Catalog {
    catalog(vec![structure(0,"root",NodeType::Section,None),Syntax::Effect(syntaxes::Effect {
        common:common_structure(1,"noop","noop","noop"),
    })])
}

fn run(catalog:&Catalog, env:&mut ReuseEnvironment) -> Result<StructureDocument,skript_parser::StructureParseError> {
    run_source(catalog,env,"root:\n    noop\n")
}

fn run_source(catalog:&Catalog, env:&mut ReuseEnvironment, source:&str) -> Result<StructureDocument,skript_parser::StructureParseError> {
    let source=MappedSource::identity(source);
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));
    parse_structures(catalog,StructureParseRequest {source:&source,tree:&tree,context:ExpressionParseContext::default()},env,StructureParserConfig::default())
}

#[test]
fn nested_body_replay_republishes_enter_context_and_rejects_incomplete_owners() {
    let mut syntaxes=fixture().syntaxes().to_vec();
    syntaxes.push(Syntax::Section(syntaxes::Section {common:common_structure(2,"branch","branch","branch"),loop_section:false,effect_section:false}));
    let catalog=catalog(syntaxes);
    let source="root:\n    branch:\n        branch:\n            noop\n";
    let mut env=ReuseEnvironment::default();
    let cold=run_source(&catalog,&mut env,source).unwrap();
    assert!(cold.diagnostics.is_empty());
    let StructureBody::Trigger(body)=&selected(&cold).body else {panic!("trigger body")};
    let skript_parser::SectionBodyNode::Section(section)=&body[0] else {panic!("nested section")};
    assert_eq!(section.selected.as_ref().unwrap().metadata.get("phase").map(String::as_str),Some("exit"));
    let calls=env.effects;
    let notices=std::mem::take(&mut env.notices);
    assert_eq!(notices.len(),2);
    assert_eq!(env.cached.as_ref().unwrap().preparations.len(),2);
    assert_eq!(run_source(&catalog,&mut env,source).unwrap(),cold);
    assert_eq!(env.effects,calls);
    assert_eq!(env.notices,notices,"actual enter proposals, not exit metadata, are republished");
    for case in 0..3 {
        let mut broken=ReuseEnvironment {cached:env.cached.clone(),..Default::default()};
        let replay=broken.cached.as_mut().unwrap();
        match case {0=>replay.preparations.clear(),1=>{replay.preparations.pop();},_=>replay.preparations.swap(0,1)}
        assert_eq!(run_source(&catalog,&mut broken,source).unwrap(),cold);
        assert!(broken.effects>0,"incomplete/misordered owners must run cold: {case}");
    }
}

#[test]
fn nested_body_replay_cancellation_and_parent_rejection_keep_prior_cache_only() {
    let mut syntaxes=fixture().syntaxes().to_vec();
    syntaxes.push(Syntax::Section(syntaxes::Section {common:common_structure(2,"branch","branch","branch"),loop_section:false,effect_section:false}));
    let catalog=catalog(syntaxes);
    let source="root:\n    branch:\n        branch:\n            noop\n";
    for cancel in [false,true] {
        let mut env=ReuseEnvironment::default();
        let baseline=run_source(&catalog,&mut env,source).unwrap();
        let cached=format!("{:?}",env.cached);
        let effects=env.effects;
        env.notices.clear();env.adoptions.clear();
        env.reject_exit=!cancel;
        env.cancel_at=cancel.then_some("publication");
        let result=run_source(&catalog,&mut env,source);
        assert_eq!(env.effects,effects,"must reach failure through a real replay, not cold body work");
        assert!(env.pending.is_none());
        assert_eq!(format!("{:?}",env.cached),cached,"failed pending must not replace prior accepted cache");
        if cancel {
            assert!(skript_parser::parse_cancellation(&result.unwrap_err()).is_some());
            assert_eq!(env.notices.iter().filter(|event|event.starts_with("section:")).count(),1,"stop between nested notifications");
            assert!(env.notices.iter().any(|event|event=="invalid:2"));
        } else {
            let rejected=result.unwrap();
            assert_eq!(env.adoptions,[false]);
            assert_eq!(env.exits,2,"parent ExitBody must rerun on replay");
            assert_eq!(env.notices.last().map(String::as_str),Some("invalid:1"));
            let mut cold=ReuseEnvironment {reject_exit:true,..Default::default()};
            assert_eq!(run_source(&catalog,&mut cold,source).unwrap(),rejected);
            assert_eq!(cold.notices,env.notices);
        }
        env.cancel_at=None;env.cancelled=false;env.reject_exit=false;env.notices.clear();
        assert_eq!(run_source(&catalog,&mut env,source).unwrap(),baseline);
        assert_eq!(env.effects,effects,"only the prior accepted cache may still be reused");
    }
}

#[test]
fn body_admission_measurement_preserves_short_circuit_and_semantic_callback_order() {
    for case in 0..5 {
        let catalog = if case == 3 {
            catalog(vec![structure(0,"root",NodeType::Section,None),structure(2,"root",NodeType::Section,None),
                Syntax::Effect(syntaxes::Effect { common:common_structure(1,"noop","noop","noop") })])
        } else { fixture() };
        let source=MappedSource::identity("root:\n    noop\n");
        let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));
        let mut reference = None;
        for measured in [false,true] {
            let mut env=ReuseEnvironment { measure_admission:measured, disabled:case==4,
                forced_mode:(case==1).then_some(skript_parser::StructureBodyMode::Raw), ..ReuseEnvironment::default() };
            let mut config=StructureParserConfig::default();
            config.expression.matcher.recover_type_expression_failures=case==2;
            let result=parse_structures(&catalog,StructureParseRequest {source:&source,tree:&tree,context:ExpressionParseContext::default()},&mut env,config).unwrap();
            let calls=(env.effects,env.prepares,env.exits,env.adoptions.clone(),env.semantic_order.clone());
            if let Some((result_before,calls_before))=&reference {assert_eq!(&result,result_before);assert_eq!(&calls,calls_before);}
            else {reference=Some((result,calls));}
            assert_eq!(env.enabled_calls.get(),usize::from(case==0 || case==4));
            if measured {
                let counts=env.admission_counts.borrow();
                assert_eq!(counts.get("body_admission.native.total"),Some(&1));
                let outcome=match case {0=>"passed",1=>"reject.mode",2=>"reject.recovery",3=>"reject.initial_ambiguity",_=>"reject.environment"};
                assert_eq!(counts.get(format!("body_admission.native.{outcome}").as_str()),Some(&1));
                assert_eq!(counts.values().sum::<usize>(),2,"one outcome per physical guardpoint");
            } else { assert!(env.admission_counts.borrow().is_empty()); }
        }
    }
}

#[test]
fn body_reuse_matches_cold_result_and_keeps_current_exit_and_adoption() {
    let mut env=ReuseEnvironment::default();
    let cold=run(&fixture(),&mut env).unwrap();
    assert!(cold.diagnostics.is_empty());
    let calls=env.effects;
    assert!(calls>0);
    let replay=run(&fixture(),&mut env).unwrap();
    assert_eq!(cold,replay);
    assert_eq!(env.effects,calls,"hit must actually skip body work");
    assert_eq!(env.exits,2,"ExitBody is always current");
    assert_eq!(env.adoptions,[true,true]);
}

#[test]
fn body_reuse_rejects_stale_counters_and_runs_the_cold_body() {
    let mut env=ReuseEnvironment::default();
    let cold=run(&fixture(),&mut env).unwrap();
    let calls=env.effects;
    env.cached.as_mut().unwrap().before.candidates+=1;
    assert_eq!(run(&fixture(),&mut env).unwrap(),cold);
    assert!(env.effects>calls);
}

#[test]
fn body_reuse_checks_layout_before_provider_admission() {
    let mut env=ReuseEnvironment::default();
    run(&fixture(),&mut env).unwrap();
    env.bad_layout=true;
    let prepares=env.prepares;
    assert!(run(&fixture(),&mut env).is_err());
    assert_eq!(env.prepares,prepares);
}

#[test]
fn body_reuse_cancellation_at_callbacks_rejects_adoption() {
    for phase in ["prepare","admit","finish","exit","semantic_finish","adopt"] {
        let mut env=ReuseEnvironment::default();
        run(&fixture(),&mut env).unwrap();
        env.adoptions.clear();
        env.cancel_at=Some(phase);
        let error=run(&fixture(),&mut env).unwrap_err();
        assert!(skript_parser::parse_cancellation(&error).is_some(),"{phase}: {error}");
        assert_eq!(env.adoptions,[phase=="adopt"],"{phase}");
    }
}

#[test]
fn body_reuse_does_not_observe_the_last_ambiguous_header_retry() {
    let mut second=structure(1,"root",NodeType::Section,None);
    let Syntax::Structure(value)=&mut second else {unreachable!()};
    value.common.registration_id=RegistrationId("other".into());
    value.common.definition_id=DefinitionId("other".into());
    let catalog=catalog(vec![structure(0,"root",NodeType::Section,None),second,Syntax::Effect(syntaxes::Effect {
        common:common_structure(2,"noop","noop","noop"),
    })]);
    let mut env=ReuseEnvironment::default();
    run(&catalog,&mut env).unwrap();
    assert_eq!(env.prepares,0);
}
