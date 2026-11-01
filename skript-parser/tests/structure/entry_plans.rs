use super::*;
use skript_parser::*;

fn entry(key:&str,kind:EntryKind)->EntryData {let mut value=key_value(key,true,false);value.kind=kind;value}
fn model()->Catalog {catalog(vec![
    structure(0,"root",NodeType::Section,Some(EntryValidator {entry_data:vec![entry("get",EntryKind::Section),entry("set",EntryKind::Unknown)]})),
    structure(1,"after",NodeType::Section,None),
    Syntax::Effect(syntaxes::Effect {common:common_structure(2,"probe","probe","probe")}),
])}
fn wire(root:u64,node:u64,index:usize,adopt:bool,event:&str)->serde_json::Value {
    json!({"version":1,"token":"17","structureNode":root,"entries":[{"node":node,"declarationIndex":index,
        "mode":"trigger","adoptUnknown":adopt,"context":{"eventClasses":[event],"values":[{"key":"parser.delay-state","value":"false"}]}}]})
}
fn header(source:&MappedSource,tree:&RawTree,catalog:&Catalog)->StructureCandidate {
    let document=parse_structures(catalog,StructureParseRequest {source,tree,context:ExpressionParseContext::default()},
        &mut NoopExpressionEnvironment,StructureParserConfig {headers_only:true,..Default::default()}).unwrap();
    document.roots.iter().find_map(|node|match node {StructureDocumentNode::Structure(matches)=>matches.selected.clone(),_=>None}).expect("current header")
}
fn attach(candidate:&mut StructureCandidate,value:serde_json::Value) {
    candidate.metadata.insert(STRUCTURE_ENTRY_LAYOUT_METADATA.into(),structure_entry_layout(candidate,"17").unwrap());
    candidate.metadata.insert(format!("example.owner/{STRUCTURE_ENTRY_PLANS_METADATA}"),value.to_string());
}

#[test]
fn entry_plans_validate_current_typed_unknown_sections_and_share_native_selection() {
    let source=MappedSource::identity("# Unicode 日本語 🦀\nroot:\n    get:\n        probe\n    set number:\n        probe\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let catalog=model();
    let mut candidate=header(&source,&tree,&catalog);let root=tree.get(candidate.raw_node_id).unwrap();
    let get=root.children[0];let set=root.children[1];
    let mut value=wire(root.id.get(),get.get(),0,false,"test.Get");
    value["entries"].as_array_mut().unwrap().push(wire(root.id.get(),set.get(),1,true,"test.Change")["entries"][0].clone());
    attach(&mut candidate,value);
    let plans=validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries).unwrap();
    let validator=candidate.entry_validator.as_ref().unwrap();
    assert_eq!(structure_entry_index(validator,tree.get(set).unwrap(),&Default::default()),None);
    assert_eq!(structure_entry_index_with_plans(validator,tree.get(set).unwrap(),&Default::default(),&plans),Some(1));
    assert_eq!(plans.entries[&set].context.event_classes,Some(vec![ClassName("test.Change".into())]));
    let mut old=candidate.clone();old.metadata.get_mut(&format!("example.owner/{STRUCTURE_ENTRY_PLANS_METADATA}")).unwrap().push(' ');
    assert!(validated_structure_entry_plans(&old,&tree,StructureBodyMode::Entries).is_ok(),"whitespace is not semantics");
}

#[test]
fn entry_plans_reject_stale_foreign_duplicate_or_unsupported_claims_before_body_work() {
    let source=MappedSource::identity("root:\n    get:\n        probe\n    set number:\n        probe\nafter:\n    probe\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let catalog=model();
    let base=header(&source,&tree,&catalog);let root=tree.get(base.raw_node_id).unwrap();let node=root.children[1];
    let good=wire(root.id.get(),node.get(),1,true,"test.Change");
    for case in 0..12 {
        let mut candidate=base.clone();let mut value=good.clone();
        match case {
            0=>value["token"]="old".into(),1=>value["structureNode"]=u64::MAX.into(),
            2=>value["entries"][0]["node"]=tree.roots[1].get().into(),
            3=>value["entries"][0]["node"]=tree.get(node).unwrap().children[0].get().into(),
            4=>value["entries"][0]["declarationIndex"]=0.into(),
            5=>value["entries"][0]["adoptUnknown"]=false.into(),
            6=>value["entries"].as_array_mut().unwrap().push(good["entries"][0].clone()),
            7=>value["entries"][0]["mode"]="raw".into(),
            8=>value["entries"][0]["context"]["values"].as_array_mut().unwrap().push(good["entries"][0]["context"]["values"][0].clone()),
            9=>value["entries"][0]["context"]["values"][0]["value"]="x".repeat(1025).into(),
            10=>value["entries"][0]["context"]["syntaxContext"]=99.into(),
            _=>value["version"]=2.into(),
        }
        attach(&mut candidate,value);
        assert!(validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries).is_err(),"case {case}");
    }
    let mut multi=base.clone();attach(&mut multi,good.clone());
    multi.metadata.insert(format!("other/{STRUCTURE_ENTRY_PLANS_METADATA}"),good.to_string());
    assert!(validated_structure_entry_plans(&multi,&tree,StructureBodyMode::Entries).is_err());
    assert!(validated_structure_entry_plans(&multi,&tree,StructureBodyMode::Trigger).is_err());
}

#[test]
fn entry_plans_never_adopt_a_native_single_duplicate_or_steal_a_known_entry() {
    for (text,claimed,index) in [("root:\n    get:\n        probe\n    get:\n        probe\n",1,1),
        ("root:\n    get:\n        probe\n",0,1),("root:\n    set number:\n        probe\n    set string:\n        probe\n",1,1)] {
        let source=MappedSource::identity(text);let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let catalog=model();
        let mut candidate=header(&source,&tree,&catalog);let root=tree.get(candidate.raw_node_id).unwrap();
        let mut value=wire(root.id.get(),root.children[claimed].get(),index,true,"test.Change");
        if text.contains("set number") {value["entries"].as_array_mut().unwrap().push(wire(root.id.get(),root.children[0].get(),1,true,"test.Change")["entries"][0].clone());}
        attach(&mut candidate,value);assert!(validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries).is_err(),"{text}");
    }
}

#[derive(Default)]
struct Environment {seen:Vec<ExpressionParseContext>,reject:bool}
impl PatternMatchEnvironment for Environment {
    fn resolve_type(&mut self,_:TypeExpressionRequest<'_>)->Result<TypeExpressionOutcome,String>{Ok(Default::default())}
    fn dispatch_hook(&mut self,_:PatternHookEvent<'_>)->Result<PatternHookControl,String>{Ok(PatternHookControl::Continue)}
}
impl ExpressionParseEnvironment for Environment {
    fn state_revision(&self)->Result<u64,String>{Ok(0)}
    fn parse_expression_leaf(&mut self,_:ExpressionLeafRequest<'_>)->Result<ExpressionLeafParse,String>{Ok(Default::default())}
    fn enter_structure(&mut self,request:StructureHookRequest<'_>)->Result<StructureHookDecision,String> {
        let mut candidate=request.candidate.clone();
        let mode=if candidate.entry_validator.is_some() {
            let root=request.tree.get(candidate.raw_node_id).unwrap();
            let mut value=wire(root.id.get(),root.children[0].get(),0,false,"test.Get");
            value["entries"].as_array_mut().unwrap().push(wire(root.id.get(),root.children[1].get(),1,true,"test.Change")["entries"][0].clone());
            attach(&mut candidate,value);StructureBodyMode::Entries
        }else{StructureBodyMode::Trigger};
        Ok(StructureHookDecision::Accept {context:request.context.clone(),body_mode:mode,entry_body_modes:BTreeMap::new(),metadata:candidate.metadata})
    }
    fn resolve_effect_candidate(&mut self,request:EffectSemanticRequest<'_>)->Result<EffectSemanticDecision,String> {
        self.seen.push(request.context.clone());
        if self.reject && request.context.event_classes==vec![ClassName("test.Get".into())] {
            return Ok(EffectSemanticDecision::Reject {reason:"fixture rejects this body statement".into(),diagnostics:vec![]});
        }
        let mut context=request.context.clone();context.values.insert("parser.delay-state".into(),"true".into());
        Ok(EffectSemanticDecision::Accepted {context,handler:None,metadata:BTreeMap::new()})
    }
}

#[test]
fn entry_plans_scope_get_change_and_following_structure_even_after_rejection() {
    for reject in [false,true] {
        let source=MappedSource::identity("root:\n    get:\n        probe\n    set number:\n        probe\nafter:\n    probe\n");
        let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let mut env=Environment {reject,..Default::default()};
        let mut context=ExpressionParseContext::default();context.event_classes=vec![ClassName("test.Outer".into())];context.syntax_context=42;
        context.values.insert("parser.delay-state".into(),"false".into());
        let result=parse_structures(&model(),StructureParseRequest {source:&source,tree:&tree,context},&mut env,StructureParserConfig::default()).unwrap();
        assert_eq!(result.diagnostics.is_empty(),!reject);
        assert_eq!(env.seen.len(),3);
        for (context,event) in env.seen.iter().zip(["test.Get","test.Change","test.Outer"]) {
            assert_eq!(context.event_classes,vec![ClassName(event.into())]);assert_eq!(context.syntax_context,42);
            assert_eq!(context.values.get("parser.delay-state").map(String::as_str),Some("false"));
        }
    }
}


#[test]
fn raw_data_adoption_is_current_and_cannot_smuggle_a_context_patch() {
    let source=MappedSource::identity("root:\n    set values:\n        first alias\n        second alias\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let catalog=model();
    let base=header(&source,&tree,&catalog);let root=tree.get(base.raw_node_id).unwrap();
    let mut good=wire(root.id.get(),root.children[0].get(),1,true,"unused");
    good["entries"][0]["mode"]="raw".into();good["entries"][0]["context"]=json!({"eventClasses":null,"values":[]});
    let mut candidate=base.clone();attach(&mut candidate,good.clone());
    let plans=validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries).unwrap();
    assert_eq!(plans.entries[&root.children[0]].mode,StructureEntryBodyMode::Raw);
    for context in [json!({"eventClasses":[],"values":[]}),json!({"eventClasses":null,"values":[{"key":"foreign","value":"injected"}]})] {
        let mut bad=good.clone();bad["entries"][0]["context"]=context;let mut candidate=base.clone();attach(&mut candidate,bad);
        assert!(validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries).is_err());
    }
}
