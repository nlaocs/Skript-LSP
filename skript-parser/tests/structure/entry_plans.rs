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
struct Environment {seen:Vec<ExpressionParseContext>,reject:bool,fatal_get:bool,raw_set:bool,scopes:Vec<(u64,bool,bool)>,scope_contexts:Vec<ExpressionParseContext>}
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
            if self.raw_set {value["entries"][1]["mode"]="raw".into();value["entries"][1]["context"]=json!({"eventClasses":null,"values":[]});}
            attach(&mut candidate,value);StructureBodyMode::Entries
        }else{StructureBodyMode::Trigger};
        Ok(StructureHookDecision::Accept {context:request.context.clone(),body_mode:mode,entry_body_modes:BTreeMap::new(),metadata:candidate.metadata})
    }
    fn enter_structure_entry_scope(&mut self,request:StructureEntryScopeRequest<'_>)->Option<u64> {
        assert_eq!(request.owner,"example.owner");assert_eq!(request.native_lease,"17");assert_eq!(request.mode,SectionBodyMode::Trigger);
        assert_eq!(request.tree.get(request.entry).unwrap().parent,Some(request.root));
        self.scope_contexts.push(request.context.clone());self.scopes.push((request.entry.get(),true,true));Some(request.entry.get())
    }
    fn exit_structure_entry_scope(&mut self,ticket:Option<u64>,completed:bool) {
        if let Some(id)=ticket {self.scopes.push((id,false,completed));}
    }
    fn resolve_effect_candidate(&mut self,request:EffectSemanticRequest<'_>)->Result<EffectSemanticDecision,String> {
        self.seen.push(request.context.clone());
        if self.fatal_get&&request.context.event_classes==vec![ClassName("test.Get".into())] {return Err("fixture fatal body failure".into());}
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
        assert_eq!(env.scopes.len(),4);assert_eq!(env.scope_contexts.len(),2);
        assert_eq!(env.scopes[1],(env.scopes[0].0,false,true));assert_eq!(env.scopes[3],(env.scopes[2].0,false,true));
        assert_eq!(env.scope_contexts[0].event_classes,vec![ClassName("test.Get".into())]);
        assert_eq!(env.scope_contexts[1].event_classes,vec![ClassName("test.Change".into())]);
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


#[test]
fn native_entry_scope_is_closed_before_a_fatal_body_error_propagates() {
    let source=MappedSource::identity("root:\n    get:\n        probe\n    set number:\n        probe\nafter:\n    probe\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let mut env=Environment {fatal_get:true,..Default::default()};
    let result=parse_structures(&model(),StructureParseRequest {source:&source,tree:&tree,context:ExpressionParseContext::default()},&mut env,StructureParserConfig::default());
    assert!(result.is_err(),"the fatal environment failure cannot silently become a complete parse");
    assert_eq!(env.scopes.len(),2);assert_eq!(env.scopes[1],(env.scopes[0].0,false,false));
}

#[test]
fn raw_entry_data_never_creates_an_executable_semantic_scope() {
    let source=MappedSource::identity("root:\n    get:\n        probe\n    set values:\n        first alias\n        second alias\nafter:\n    probe\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let mut env=Environment {raw_set:true,..Default::default()};
    let mut context=ExpressionParseContext::default();context.event_classes=vec![ClassName("test.Outer".into())];
    let result=parse_structures(&model(),StructureParseRequest {source:&source,tree:&tree,context},&mut env,StructureParserConfig::default()).unwrap();
    assert!(result.diagnostics.is_empty());assert_eq!(env.scopes.len(),2);assert_eq!(env.seen.len(),2);
    assert_eq!(env.seen[1].event_classes,vec![ClassName("test.Outer".into())]);
}


fn alias_wire(root:RawNodeId,node:RawNodeId,alias:&str)->serde_json::Value {
    json!({"version":1,"token":"17","structureNode":root.get(),"entries":[{"node":node.get(),"declarationIndex":0,
        "mode":"raw","adoptUnknown":true,"keyAlias":alias,"context":{"eventClasses":null,"values":[]}}]})
}
#[test]
fn key_alias_admission_rejects_foreign_context_kind_separator_and_source_claims() {
    let source=MappedSource::identity("root:\n    値😀: 123\n    nested:\n        値😀: 456\nafter:\n    値😀: 789\n");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let catalog=model();
    let mut base=header(&source,&tree,&catalog);base.entry_validator=Some(EntryValidator {entry_data:vec![entry("canonical",EntryKind::KeyValue)]});
    let root=tree.get(base.raw_node_id).unwrap();let node=root.children[0];let good=alias_wire(root.id,node,"値😀");
    let mut current=base.clone();attach(&mut current,good.clone());
    let plans=validated_structure_entry_plans(&current,&tree,StructureBodyMode::Entries).unwrap();
    assert_eq!(structure_entry_index_with_plans(current.entry_validator.as_ref().unwrap(),tree.get(node).unwrap(),&Default::default(),&plans),Some(0));
    for case in 0..17 {
        let mut candidate=base.clone();let mut value=good.clone();
        match case {
            0=>value["token"]="stale".into(),
            1=>value["entries"][0]["node"]=tree.get(root.children[1]).unwrap().children[0].get().into(),
            2=>value["entries"][0]["node"]=tree.get(tree.roots[1]).unwrap().children[0].get().into(),
            3=>value["entries"][0]["node"]=root.children[1].get().into(),
            4=>candidate.entry_validator.as_mut().unwrap().entry_data[0].kind=EntryKind::Expression,
            5=>candidate.entry_validator.as_mut().unwrap().entry_data[0].separator=Some(" = ".into()),
            6=>value["entries"][0]["keyAlias"]="値".into(),
            7=>value["entries"][0]["keyAlias"]="canonical".into(),
            8=>value["entries"][0]["keyAlias"]="".into(),
            9=>value["entries"][0]["keyAlias"]="x".repeat(257).into(),
            10=>value["entries"][0]["keyAlias"]="値\n".into(),
            11=>value["entries"][0]["mode"]="trigger".into(),
            12=>value["entries"][0]["adoptUnknown"]=false.into(),
            13=>value["entries"][0]["context"]["eventClasses"]=json!([]),
            14=>value["entries"][0]["context"]["values"]=json!([{"key":"scope","value":"changed"}]),
            15=>candidate.entry_validator.as_mut().unwrap().entry_data.push(entry("値😀",EntryKind::KeyValue)),
            _=>{value["entries"].as_array_mut().unwrap().push(good["entries"][0].clone());},
        }
        attach(&mut candidate,value);
        assert!(validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries).is_err(),"case {case}");
    }
}
#[test]
fn key_alias_and_canonical_spelling_share_single_or_multiple_cardinality_in_both_orders() {
    for multiple in [false,true] {for spellings in [["alias","canonical"],["canonical","alias"],["alias","alias"]] {
        let source=MappedSource::identity(format!("root:\n    {}: first\n    {}: second\n",spellings[0],spellings[1]));
        let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));let catalog=model();
        let mut candidate=header(&source,&tree,&catalog);let mut data=entry("canonical",EntryKind::KeyValue);data.multiple=multiple;
        candidate.entry_validator=Some(EntryValidator {entry_data:vec![data]});let root=tree.get(candidate.raw_node_id).unwrap();
        let mut value=alias_wire(root.id,root.children[0],"alias");value["entries"]=json!([]);
        for (i,key) in spellings.iter().enumerate() {if *key=="alias" {value["entries"].as_array_mut().unwrap().push(alias_wire(root.id,root.children[i],key)["entries"][0].clone());}}
        attach(&mut candidate,value);
        let plans=validated_structure_entry_plans(&candidate,&tree,StructureBodyMode::Entries);
        if !multiple&&spellings[1]=="alias" {assert!(plans.is_err());continue;}
        let plans=plans.unwrap();let validator=candidate.entry_validator.as_ref().unwrap();let mut seen=Default::default();
        assert_eq!(structure_entry_index_with_plans(validator,tree.get(root.children[0]).unwrap(),&seen,&plans),Some(0));seen.insert(0);
        assert_eq!(structure_entry_index_with_plans(validator,tree.get(root.children[1]).unwrap(),&seen,&plans),multiple.then_some(0));
    }}
}
