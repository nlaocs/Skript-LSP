use super::*;
use crate::lsp::semantic::{guards, transfer::{Resolver, TypeBound, Output, SourceUpdate, BindingEffects}};

// An addon-authored predicate, not a spelling recognized by variable analysis.
struct Reads {mode: &'static str, latest:Option<u64>, calls:Vec<u64>}
impl Resolver for Reads {
    fn binding_identity(&self,id:u64)->bool {self.mode=="identity"&&self.latest==Some(id)}
    fn evaluate(&mut self,id:u64,input:TypeBound)->Result<Option<Output>,String> {
        self.latest=Some(id);self.calls.push(id);
        Ok(Some(Output {value:input.clone(),source:match self.mode {
            "known"=>SourceUpdate::Known(input),"unknown"=>SourceUpdate::Unknown,_=>SourceUpdate::Unchanged},
            source_writeback:None,effects:BindingEffects::Preserve}))
    }
}
fn read(f:&mut Fixture,row:usize)->u64 {
    let id=f.var(row,"{_value}","value",true,false);
    f.nodes[id as usize].metadata.retain(|m|m.key!=VARIABLE_READ);
    f.nodes[id as usize].metadata.extend([
        metadata(VARIABLE_READ,json!({"version":3,"requestedTypes":["java.lang.Object"],"mayRefreshPlayer":true,"noRefreshTypes":[],"missingValue":"empty"})),
        metadata(parser_wasm::host::semantic_transfer::METADATA,json!({"version":3,"operation":"authored.read","parameters":"{}","outputUse":"flow","valueRelation":"same-source-when-unchanged","bindingValues":"preserve"})),
    ]);id
}
fn init(f:&mut Fixture,row:usize,types:&[&str],nullable:bool)->u64 {
    let id=f.assign(row,"{_value}","value","initial",types[0],true,false,"set");
    f.nodes[id as usize].metadata.retain(|m|m.key!=CHANGE);
    f.nodes[id as usize].metadata.push(metadata(CHANGE,json!({"version":3,"mode":"set","targetCapture":0,"valueCapture":1,
        "storageResult":{"version":1,"scope":"local-scalar","types":types,"complete":true,"multiple":false,"maybeUnset":nullable,"preservesLocalBindings":true}})));
    id
}
fn condition(f:&mut Fixture,row:usize,subject:u64,predicate:serde_json::Value)->u64 {
    let id=f.push(SyntaxKind::Condition,"condition",f.line(row));f.seal(id,&[(0,subject)]);
    f.nodes[id as usize].metadata.extend([
        metadata(guards::GUARD,predicate),metadata(guards::VALUES,json!({"version":1,"kind":"preserve"})),
        metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),metadata(OPERANDS,json!({"version":1,"prefix":[0]})),
    ]);id
}
fn section(f:&mut Fixture,row:usize,condition:u64,body:&[u64])->u64 {
    let id=f.container(row,SyntaxKind::Section,body,Some("branch"));f.seal(id,&[(0,condition)]);
    f.nodes[id as usize].metadata.retain(|m|m.key!=FLOW);
    f.nodes[id as usize].metadata.extend([
        metadata(FLOW,json!({"version":2,"kind":"branch","conditionCapture":0,"bodyWhen":true})),
        metadata(OPERANDS,json!({"version":1,"prefix":[0]})),metadata(guards::VALUES,json!({"version":1,"kind":"preserve"})),
    ]);id
}
fn inspect(f:&mut Fixture,row:usize)->u64 {
    let value=read(f,row);let id=f.push(SyntaxKind::Effect,"effect",f.line(row));f.seal(id,&[(0,value)]);
    f.nodes[id as usize].metadata.extend([metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),metadata(OPERANDS,json!({"version":1,"prefix":[0]})),metadata(crate::lsp::semantic::shared::BINDINGS,json!({"version":1,"local":"preserve","shared":"preserve"}))]);id
}
fn finish(f:Fixture,root:u64,reads:&mut Reads)->Document {
    let mapped=skript_parser::MappedSource::identity(Arc::clone(&f.source));
    let raw=skript_parser::parse_raw_tree(&mapped,skript_parser::RawTreeOptions::for_skript_version(2,15));
    Document::from_ast(f.source,"file:///guards.sk".parse().unwrap(),crate::lsp::test_support::runtime_catalog(),
        &AstTree {roots:vec![root],nodes:f.nodes},&raw,Vec::new(),Vec::new(),None,None,HashSet::new(),Vec::new(),Some(reads)).unwrap()
}
fn observed(d:&Document,row:u32)->&InferredType {d.type_at(Position::new(row,18)).unwrap().1}
fn presence(positive:bool)->serde_json::Value {
    json!({"version":1,"subjectCapture":0,"whenTrue":{"presence":if positive {"present"}else{"absent"}},
        "whenFalse":{"presence":if positive {"absent"}else{"present"}}})
}
#[test]
fn current_nullable_local_is_refined_only_inside_the_justified_branch() {
    for positive in [true,false] {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        inspect {_value}\n    inspect {_value}");
        let set=init(&mut f,1,&["java.lang.String"],true);let subject=read(&mut f,2);
        let c=condition(&mut f,2,subject,presence(positive));let inside=inspect(&mut f,3);let branch=section(&mut f,2,c,&[inside]);
        let after=inspect(&mut f,4);let root=f.container(0,SyntaxKind::Structure,&[set,branch,after],None);
        let mut reads=Reads {mode:"identity",latest:None,calls:Vec::new()};let d=finish(f,root,&mut reads);
        assert!(matches!(observed(&d,3),InferredType::Values {classes,maybe_unset,..}if *maybe_unset==!positive&&classes.is_empty()==!positive));
        assert!(matches!(observed(&d,4),InferredType::Values {maybe_unset:true,..}),"branch exit must join the bypass");
        assert!(!reads.calls.is_empty());
        assert!(matches!(observed(&d,3),InferredType::Values {support,..}if support.evidence.contains(&Evidence::TypeGuard)));
    }
}
#[test]
fn same_bound_refresh_or_legacy_transfer_does_not_prove_checked_binding_identity() {
    for mode in ["legacy","known","unknown"] {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        inspect {_value}");
        let set=init(&mut f,1,&["java.lang.String"],true);let subject=read(&mut f,2);let c=condition(&mut f,2,subject,presence(true));
        let inside=inspect(&mut f,3);let branch=section(&mut f,2,c,&[inside]);let root=f.container(0,SyntaxKind::Structure,&[set,branch],None);
        let mut reads=Reads {mode,latest:None,calls:Vec::new()};let d=finish(f,root,&mut reads);
        assert!(!matches!(observed(&d,3),InferredType::Values {maybe_unset:false,..}),"{mode}");
    }
}
#[test]
fn addon_bound_narrows_a_current_complete_union_but_unknown_overlap_stays_open() {
    for (types,expected) in [(vec!["java.lang.String","java.lang.Long"],vec!["java.lang.String"]),
        (vec!["java.lang.String","unseen.Interface"],vec!["java.lang.String","unseen.Interface"]),
        (vec!["unseen.Fresh"],vec!["unseen.Fresh"])] {
        let target=if types==["unseen.Fresh"] {"unseen.Fresh"}else{"java.lang.String"};
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        inspect {_value}");
        let set=init(&mut f,1,&types,true);let subject=read(&mut f,2);
        let c=condition(&mut f,2,subject,json!({"version":1,"subjectCapture":0,"whenTrue":{"presence":"present","upperBounds":[target]},"whenFalse":{"presence":"keep"}}));
        let inside=inspect(&mut f,3);let branch=section(&mut f,2,c,&[inside]);let root=f.container(0,SyntaxKind::Structure,&[set,branch],None);
        let mut reads=Reads {mode:"identity",latest:None,calls:Vec::new()};let d=finish(f,root,&mut reads);
        assert!(matches!(observed(&d,3),InferredType::Values {classes,maybe_unset:false,..}if *classes==expected.iter().map(|s|s.to_string()).collect()),"{types:?}: {:?}",observed(&d,3));
    }
}
#[test]
fn predicate_and_static_operand_evidence_require_current_matching_owners_and_geometry() {
    for bad in ["valid","foreign-guard","duplicate","foreign-evidence","missing-evidence","changed-token","bad-capture","version"] {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked StaticType:\n        inspect {_value}");
        let set=init(&mut f,1,&["java.lang.String"],true);let subject=read(&mut f,2);let mut predicate=presence(true);
        predicate["evidence"]=json!([{"capture":1,"token":"declared.Class"}]);if bad=="version" {predicate["version"]=2.into();}
        let c=condition(&mut f,2,subject,predicate);let rhs=f.push(SyntaxKind::Expression,"literal",f.part(2,"StaticType"));
        f.nodes[rhs as usize].metadata.extend([metadata(guards::VALUES,json!({"version":1,"kind":"preserve"})),metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),metadata(guards::EVIDENCE,json!({"version":1,"token":if bad=="changed-token" {"other.Class"}else{"declared.Class"}}))]);
        f.seal(c,&[(1,rhs)]);f.nodes[c as usize].metadata.retain(|m|m.key!=OPERANDS);f.nodes[c as usize].metadata.push(metadata(OPERANDS,json!({"version":1,"prefix":[0,1]})));
        if bad=="foreign-guard" {f.nodes[c as usize].metadata.iter_mut().find(|m|m.key==guards::GUARD).unwrap().owner_component_id=Some("other".into());}
        if bad=="duplicate" {let copy=f.nodes[c as usize].metadata.iter().find(|m|m.key==guards::GUARD).unwrap().clone();f.nodes[c as usize].metadata.push(copy);}
        if bad=="foreign-evidence" {f.nodes[rhs as usize].metadata.iter_mut().find(|m|m.key==guards::EVIDENCE).unwrap().owner_component_id=Some("other".into());}
        if bad=="missing-evidence" {f.nodes[rhs as usize].metadata.retain(|m|m.key!=guards::EVIDENCE);}
        if bad=="bad-capture" {f.nodes[c as usize].captures.retain(|c|c.name!="parser.pattern-capture:1");}
        for id in [c,rhs] {retain_semantic_contracts(&mut f.nodes[id as usize],Some("fixture.semantic-provider"));}
        let inside=inspect(&mut f,3);let branch=section(&mut f,2,c,&[inside]);let root=f.container(0,SyntaxKind::Structure,&[set,branch],None);
        let mut reads=Reads {mode:"identity",latest:None,calls:Vec::new()};let d=finish(f,root,&mut reads);
        assert_eq!(matches!(observed(&d,3),InferredType::Values {maybe_unset:false,..}),bad=="valid","{bad}: {:?}",observed(&d,3));
    }
}

#[test]
fn standard_presence_guard_runs_through_the_actual_component_and_current_transfer_session() {
    for complete in [false,true] {
    let catalog=if complete {Arc::new(ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain")).unwrap().into_catalog())}else{crate::lsp::test_support::runtime_catalog()};
    let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(Arc::clone(&catalog)),
        runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()}).unwrap();
    let source:Arc<str>=Arc::from("# 雪😀\nfunction guarded(value: text):\n    if {_value} is set:\n        set {_inside} to {_value}\n    set {_outside} to {_value}\n");
    let uri:Uri="file:///guards/public.sk".parse().unwrap();
    let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///guards",uri.as_str(),1,source.as_ref()),Default::default()).unwrap();
    assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
    assert!(result.syntax.diagnostics.is_empty(),"{:?}",result.syntax.diagnostics);
    let document=Document::from_result_with_host(&result,Arc::clone(&source),uri,Arc::clone(&catalog),&mut host,None).unwrap();
    let hints=document.write_type_hints();
    for (line,label) in [(3,if complete {": string"}else{": string?"}),(4,": string?")] {
        assert!(hints.iter().any(|h|h.position.line==line&&matches!(&h.label,lsp_types::InlayHintLabel::String(actual)if actual==label)),"{hints:?}");
    }
    let c=result.ast.nodes.iter().find(|n|n.kind==SyntaxKind::Condition&&n.text.contains("is set")).unwrap();
    assert!(c.metadata.iter().any(|m|m.key==guards::GUARD));
    }
}

#[test]
fn intervening_operand_needs_value_preservation_not_only_equal_type_facts() {
    for pure in [false,true] {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked operand:\n        inspect {_value}");
        let set=init(&mut f,1,&["java.lang.String"],true);let subject=read(&mut f,2);
        let c=condition(&mut f,2,subject,presence(true));
        let rhs=f.push(SyntaxKind::Expression,"literal",f.part(2,"operand"));
        f.nodes[rhs as usize].metadata.push(metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})));
        if pure {f.nodes[rhs as usize].metadata.push(metadata(guards::VALUES,json!({"version":1,"kind":"preserve"})));}
        f.seal(c,&[(1,rhs)]);f.nodes[c as usize].metadata.retain(|m|m.key!=OPERANDS);
        f.nodes[c as usize].metadata.push(metadata(OPERANDS,json!({"version":1,"prefix":[0,1]})));
        let inside=inspect(&mut f,3);let branch=section(&mut f,2,c,&[inside]);
        let root=f.container(0,SyntaxKind::Structure,&[set,branch],None);
        let mut reads=Reads {mode:"identity",latest:None,calls:Vec::new()};let d=finish(f,root,&mut reads);
        assert_eq!(matches!(observed(&d,3),InferredType::Values {maybe_unset:false,..}),pure);
        assert_eq!(matches!(observed(&d,3),InferredType::Values {support,..}if support.evidence.contains(&Evidence::TypeGuard)),pure);
    }
}
