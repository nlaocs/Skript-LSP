use super::*;

struct Environment {cancel:bool,requested:bool}
impl crate::PatternMatchEnvironment for Environment {
    fn cancellation_requested(&mut self)->bool {self.requested}
    fn resolve_type(&mut self,_:crate::TypeExpressionRequest<'_>)->Result<crate::TypeExpressionOutcome,String>{Ok(Default::default())}
    fn dispatch_hook(&mut self,_:crate::PatternHookEvent<'_>)->Result<crate::PatternHookControl,String>{Ok(crate::PatternHookControl::Continue)}
}
impl crate::ExpressionParseEnvironment for Environment {
    fn state_revision(&self)->Result<u64,String>{Ok(0)}
    fn parse_expression_leaf(&mut self,_:crate::ExpressionLeafRequest<'_>)->Result<crate::ExpressionLeafParse,String>{Ok(Default::default())}
    fn resolve_effect_candidate(&mut self,request:crate::EffectSemanticRequest<'_>)->Result<crate::EffectSemanticDecision,String>{
        assert_eq!(request.context.event_classes,vec![ClassName("test.Entry".into())]);
        if self.cancel {self.requested=true;Ok(crate::EffectSemanticDecision::UseCandidate)}else{Err("injected handler failure".into())}
    }
}
#[test]
fn entry_context_is_restored_before_fault_or_cancellation_escapes() {
    let fixture=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
    let snapshot=ssg::load(fixture).unwrap();let catalog=snapshot.catalog();
    let source=crate::MappedSource::identity("root:\n    code:\n        stop\n        stop\n");
    let tree=crate::parse_raw_tree(&source,crate::RawTreeOptions::for_skript_version(2,15));
    let root=tree.get(tree.roots[0]).unwrap();let node=tree.get(root.children[0]).unwrap();
    let data=EntryData {key:"code".into(),default_value:None,optional:true,multiple:false,entry_data_class:ClassName("test.Entry".into()),
        kind:EntryKind::Section,separator:None,value_type:None,string_mode:None,return_types:vec![],flags:None,nested_validator:None};
    let mut original=ExpressionParseContext::default();original.syntax_context=37;original.event_classes=vec![ClassName("test.Outer".into())];
    original.values.insert("outer".into(),"retained".into());
    let plan=StructureEntryPlan {declaration_index:0,mode:crate::SectionBodyMode::Trigger,adopt_unknown:false,
        context:StructureEntryContext {event_classes:Some(vec![ClassName("test.Entry".into())]),values:BTreeMap::from([("outer".into(),None),("entry".into(),Some("local".into()))])}};
    for cancel in [false,true] {
        let mut environment=Environment {cancel,requested:false};
        let mut session=ExpressionSession::new(catalog,None,&source,&mut environment,original.clone(),Default::default());
        let parsed=super::super::parse_entry(&mut session,&tree,node,0,&data,&BTreeMap::new(),Some(&plan),&["code".into()],1);
        assert!(parsed.is_err(),"fault/cancel must escape");
        assert_eq!(session.context(),&original,"restore precedes propagation, including nullable context removals");
    }
}
