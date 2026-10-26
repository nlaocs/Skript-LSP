use super::*;
use skript_parser::{
    EffectSemanticDecision, EffectSemanticRequest, PortableStructureBody, StructureBodyPlan,
    StructureBodyReplay, StructureBodyReuseRequest, StructureBodyWork, StructureHookDecision,
};

#[derive(Default)]
struct ReuseEnvironment {
    cached: Option<StructureBodyReplay>,
    before: Option<StructureBodyWork>,
    prepares: usize,
    effects: usize,
    exits: usize,
    adoptions: Vec<bool>,
    cancel_at: Option<&'static str>,
    cancelled: bool,
    bad_layout: bool,
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
        Ok(StructureExitDecision::Accept)
    }
    fn structure_body_reuse_enabled(&self) -> bool {
        self.enabled_calls.set(self.enabled_calls.get()+1);
        !self.disabled
    }
    fn prepare_structure_body(&mut self, request: StructureBodyReuseRequest<'_>) -> Result<StructureBodyPlan, String> {
        self.prepares += 1;
        self.before=Some(request.work);
        self.cancelled |= self.cancel_at == Some("prepare");
        Ok(self.cached.clone().map_or(StructureBodyPlan::Observe,StructureBodyPlan::Reuse))
    }
    fn accept_structure_body_replay(&mut self, _: StructureBodyReuseRequest<'_>) -> Result<bool,String> {
        self.cancelled |= self.cancel_at == Some("admit");
        Ok(true)
    }
    fn finish_structure_body(&mut self, body: &mut StructureBody, context: &ExpressionParseContext, work: StructureBodyWork) -> Result<bool,String> {
        self.cached=Some(StructureBodyReplay {
            before:self.before.unwrap(), body:PortableStructureBody::freeze(body,&mut |_,_|false).unwrap(),
            post_context:context.clone(),work,preparations:Vec::new(),
        });
        self.cancelled |= self.cancel_at == Some("finish");
        Ok(true)
    }
    fn adopt_structure_body(&mut self, accepted:bool) -> Result<(),String> {
        self.adoptions.push(accepted);
        self.semantic_order.push("adopt");
        self.cancelled |= self.cancel_at == Some("adopt");
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
    let source=MappedSource::identity("root:\n    noop\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));
    parse_structures(catalog,StructureParseRequest {source:&source,tree:&tree,context:ExpressionParseContext::default()},env,StructureParserConfig::default())
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
