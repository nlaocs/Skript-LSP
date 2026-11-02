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
fn read(f:&mut Fixture,row:usize)->u64 {read_named(f,row,"{_value}","value")}
fn read_named(f:&mut Fixture,row:usize,spelling:&str,name:&str)->u64 {
    let id=f.var(row,spelling,name,true,false);
    f.nodes[id as usize].metadata.retain(|m|m.key!=VARIABLE_READ);
    f.nodes[id as usize].metadata.extend([
        metadata(VARIABLE_READ,json!({"version":3,"requestedTypes":["java.lang.Object"],"mayRefreshPlayer":true,"noRefreshTypes":[],"missingValue":"empty"})),
        metadata(parser_wasm::host::semantic_transfer::METADATA,json!({"version":3,"operation":"authored.read","parameters":"{}","outputUse":"flow","valueRelation":"same-source-when-unchanged","bindingValues":"preserve"})),
    ]);id
}
fn init(f:&mut Fixture,row:usize,types:&[&str],nullable:bool)->u64 {init_named(f,row,"{_value}","value",types,nullable)}
fn init_named(f:&mut Fixture,row:usize,spelling:&str,name:&str,types:&[&str],nullable:bool)->u64 {
    let id=f.assign(row,spelling,name,"initial",types[0],true,false,"set");
    f.nodes[id as usize].metadata.retain(|m|m.key!=CHANGE);
    f.nodes[id as usize].metadata.push(metadata(CHANGE,json!({"version":3,"mode":"set","targetCapture":0,"valueCapture":1,"targetOnly":true,
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

// Uses the same owned snapshot/data as the dedicated editor failure. It does
// not claim that an API declaration is captured runtime behavior.
#[test]
#[ignore = "requires the explicit reflect snapshot, matching candidate and owned collector fixture"]
fn reflect_guard_copy_remains_narrowed_in_the_project_capable_component() {
    use sha2::{Digest, Sha256};
    use parser_wasm::host::{AddonDataPin, AddonDataSource};
    let snapshot = std::env::var("SKRIPT_REFLECT_SNAPSHOT").unwrap();
    let candidate = std::fs::read(std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").unwrap()).unwrap();
    let bytes = std::fs::read(std::env::var("SKRIPT_REFLECT_OWNED_DATA").unwrap()).unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let data = AddonDataSource::from_json(AddonDataPin {
        component_id: "nlaocs.skript-reflect".into(),
        component_sha256: format!("{:x}", Sha256::digest(&candidate)),
        id: envelope["id"].as_str().unwrap().into(),
        schema_id: envelope["schemaId"].as_str().unwrap().into(),
        schema_version: envelope["schemaVersion"].as_u64().unwrap() as u32,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bindings: serde_json::from_value(envelope["bindings"].clone()).unwrap(),
    }, &bytes).unwrap();
    let catalog = Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
    let mut host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)), addon_data_sources: vec![data],
        ..Default::default()
    }).unwrap();
    host.load_addon(&candidate).unwrap();
    assert!(host.project_declarations_enabled());
    let body = "import:\n    java.lang.String as JavaString\non load:\n    set {_name} to \"hello\"\n    if chance of 50%:\n        set {_name} to 123\n    if {_name} is an instance of JavaString:\n        set {_copy} to {_name}\n        set {_part} to {_copy}.sub\n";
    let uri: Uri = "file:///guards/reflect-project.sk".parse().unwrap();
    for (revision, prefix) in [(1, ""), (2, "# Unicode \u{1F980}\n"), (4, "")] {
        if revision == 4 {
            let token = parser_wasm::DocumentCancellationToken::new();
            token.cancel();
            let cancelled = host.parse_document(parser_wasm::DocumentParseRequest::new(
                "file:///guards", uri.as_str(), 3, body), parser_wasm::DocumentParserConfig {
                    cancellation: token, ..Default::default()
                });
            assert!(matches!(cancelled, Err(parser_wasm::DocumentParseError::Cancelled { .. })));
        }
        let source: Arc<str> = Arc::from(format!("{prefix}{body}"));
        let result = host.parse_document(parser_wasm::DocumentParseRequest::new(
            "file:///guards", uri.as_str(), revision, source.as_ref()), Default::default()).unwrap();
        assert!(result.component_failures.is_empty(), "{:?}", result.component_failures);
        let document = Document::from_result_with_host(&result, Arc::clone(&source),
            uri.clone(), Arc::clone(&catalog), &mut host, None).unwrap();
        let line = 7 + u32::from(!prefix.is_empty());
        let hints = document.write_type_hints();
        assert!(hints.iter().any(|hint| hint.position.line == line && matches!(
            &hint.label, lsp_types::InlayHintLabel::String(label) if label == ": string")),
            "revision {revision}: {hints:?}");
    }
}


#[cfg(test)]
mod else_flow {
    use super::*;
    pub(super) fn branch(f:&mut Fixture,row:usize,condition:Option<u64>,body:&[u64])->u64 {
        let id=if let Some(condition)=condition {section(f,row,condition,body)}
            else {f.container(row,SyntaxKind::Section,body,Some("branch"))};
        f.nodes[id as usize].metadata.retain(|m|m.key!=FLOW);
        let flow=if condition.is_some() {json!({"version":3,"kind":"branch","chain":"start","conditionCapture":0,"bodyWhen":true})}
            else {json!({"version":3,"kind":"branch","chain":"else"})};
        f.nodes[id as usize].metadata.push(metadata(FLOW,flow));
        if condition.is_none() {
            f.nodes[id as usize].metadata.extend([
                metadata(OPERANDS,json!({"version":1,"prefix":[]})),
                metadata(guards::VALUES,json!({"version":1,"kind":"preserve"})),
            ]);
        }
        id
    }
    pub(super) fn run(f:Fixture,ids:&[u64])->Document {
        let mut f=f;let root=f.container(0,SyntaxKind::Structure,ids,None);
        finish(f,root,&mut Reads {mode:"identity",latest:None,calls:Vec::new()})
    }
    pub(super) fn present(d:&Document,row:u32)->bool {
        matches!(observed(d,row),InferredType::Values {maybe_unset:false,..})
    }
    #[test]
    fn false_facts_survive_effects_on_the_other_path_without_rewinding_guards() {
        for operation in ["preserve","reassign","suspend","opaque"] {
            let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        bind {_value} = initial\n    else:\n        inspect {_value}\n    inspect {_value}");
            let initial=init(&mut f,1,&["java.lang.String"],true);
            let subject=read(&mut f,2);let condition=condition(&mut f,2,subject,presence(false));
            let effect=match operation {
                "reassign"=>init(&mut f,3,&["java.lang.Long"],false),
                _=>{
                    let id=f.push(SyntaxKind::Effect,"effect",f.line(3));
                    if operation!="opaque" {
                        f.nodes[id as usize].metadata.extend([
                            metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),
                            metadata(shared::BINDINGS,json!({"version":1,"local":"preserve","shared":if operation=="suspend" {"suspend"}else{"preserve"}})),
                        ]);
                    }
                    id
                }
            };
            let start=branch(&mut f,2,Some(condition),&[effect]);
            let inside=inspect(&mut f,5);let alternate=branch(&mut f,4,None,&[inside]);
            let after=inspect(&mut f,6);let d=run(f,&[initial,start,alternate,after]);
            assert!(present(&d,5),"{operation}: {:?}",observed(&d,5));
            assert!(matches!(observed(&d,5),InferredType::Values {classes,..}if *classes==BTreeSet::from(["java.lang.String".into()])));
            if operation=="opaque" {assert!(matches!(observed(&d,6),InferredType::Unknown));}
            if operation=="reassign" {
                assert!(matches!(observed(&d,6),InferredType::Values {classes,maybe_unset:false,..}
                    if *classes==BTreeSet::from(["java.lang.Long".into(),"java.lang.String".into()])));
            }
        }
    }
    #[test]
    fn both_branch_assignments_join_without_the_nonexistent_extra_bypass() {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        bind {_value} = initial\n    else:\n        bind {_value} = initial\n    inspect {_value}");
        let initial=init(&mut f,1,&["java.lang.String"],true);
        let subject=read(&mut f,2);let c=condition(&mut f,2,subject,presence(false));
        let left=init(&mut f,3,&["java.lang.Long"],false);let start=branch(&mut f,2,Some(c),&[left]);
        let right=init(&mut f,5,&["java.lang.String"],false);let alternate=branch(&mut f,4,None,&[right]);
        let after=inspect(&mut f,6);let d=run(f,&[initial,start,alternate,after]);
        assert!(matches!(observed(&d,6),InferredType::Values {classes,maybe_unset:false,support,..}
            if *classes==BTreeSet::from(["java.lang.Long".into(),"java.lang.String".into()])
                && support.evidence.contains(&Evidence::ControlFlowJoin)));
        let occurrence=d.occurrences.iter().find(|o|d.lines.position(&d.source,o.range.start).unwrap().line==6).unwrap();
        assert!(matches!(&occurrence.reference,Reference::Variable {reaching,..} if reaching.len()==2));
    }
    #[test]
    fn incomplete_branch_assignment_does_not_initialize_the_other_path() {
        let mut f=Fixture::new("owner:\n    if {_value} checked:\n        bind {_value} = initial\n    else:\n        # empty\n    inspect {_value}");
        let subject=read(&mut f,1);let c=condition(&mut f,1,subject,presence(false));
        let left=init(&mut f,2,&["java.lang.Long"],false);let start=branch(&mut f,1,Some(c),&[left]);
        let alternate=branch(&mut f,3,None,&[]);let after=inspect(&mut f,5);
        let d=run(f,&[start,alternate,after]);assert!(matches!(observed(&d,5),InferredType::Unknown));
    }
    #[test]
    fn raw_sibling_barriers_break_chaining_but_comments_and_empty_taken_body_do_not() {
        for barrier in ["# comment","unparsed statement"] {
            let mut f=Fixture::new(&format!("owner:\n    bind {{_value}} = initial\n    if {{_value}} checked:\n        # empty\n    {barrier}\n    else:\n        inspect {{_value}}"));
            let initial=init(&mut f,1,&["java.lang.String"],true);
            let subject=read(&mut f,2);let c=condition(&mut f,2,subject,presence(false));
            let start=branch(&mut f,2,Some(c),&[]);let inside=inspect(&mut f,6);let alternate=branch(&mut f,5,None,&[inside]);
            let d=run(f,&[initial,start,alternate]);
            assert_eq!(present(&d,6),barrier=="# comment","{barrier}: {:?}",observed(&d,6));
        }
    }
    #[test]
    fn invalid_else_contract_never_selects_the_false_input() {
        for bad in ["owner","version","missing-role","operand","intervening-node","ordinary-v2"] {
            let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        # empty\n    no-op\n    else:\n        inspect {_value}");
            let initial=init(&mut f,1,&["java.lang.String"],true);
            let subject=read(&mut f,2);let c=condition(&mut f,2,subject,presence(false));
            let start=branch(&mut f,2,Some(c),&[]);
            let inside=inspect(&mut f,6);let alternate=branch(&mut f,5,None,&[inside]);
            for m in f.nodes[alternate as usize].metadata.iter_mut().filter(|m|m.key==FLOW) {
                if bad=="owner" {m.owner_component_id=Some("another.provider".into());}
                if bad=="version" {m.value=json!({"version":99,"kind":"branch","chain":"else"}).to_string();}
                if bad=="missing-role" {m.value=json!({"version":3,"kind":"branch"}).to_string();}
                if bad=="ordinary-v2" {m.value=json!({"version":2,"kind":"branch"}).to_string();}
            }
            if bad=="operand" {
                let operand=f.push(SyntaxKind::Expression,"literal",f.part(5,"else"));
                f.nodes[alternate as usize].children.push(operand);
            }
            let extra=if bad=="intervening-node" {Some(f.push(SyntaxKind::Effect,"effect",f.line(4)))}else{None};
            // No missing raw statement is intended in the schema/owner cases.
            // Remove the no-op from the raw source unless testing adjacency.
            if bad!="intervening-node" {
                let from=f.line(4);let mut source=f.source.to_string();
                source.replace_range(from.start..from.end,&" ".repeat(from.end-from.start));f.source=source.into();
            }
            let mut ids=vec![initial,start];ids.extend(extra);ids.push(alternate);
            let d=run(f,&ids);assert!(!present(&d,6),"{bad}: {:?}",observed(&d,6));
        }
    }
    #[test]
    fn nested_outcomes_do_not_replace_the_outer_false_path() {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        if {_value} checked:\n            bind {_value} = initial\n        else:\n            bind {_value} = initial\n    else:\n        inspect {_value}\n    inspect {_value}");
        let initial=init(&mut f,1,&["java.lang.String"],true);
        let subject=read(&mut f,2);let outer=condition(&mut f,2,subject,presence(false));
        let subject=read(&mut f,3);let inner=condition(&mut f,3,subject,presence(true));
        let left=init(&mut f,4,&["java.lang.Long"],false);let inner=branch(&mut f,3,Some(inner),&[left]);
        let right=init(&mut f,6,&["java.lang.String"],false);let inner_else=branch(&mut f,5,None,&[right]);
        let start=branch(&mut f,2,Some(outer),&[inner,inner_else]);
        let inside=inspect(&mut f,8);let alternate=branch(&mut f,7,None,&[inside]);
        let after=inspect(&mut f,9);let d=run(f,&[initial,start,alternate,after]);
        assert!(matches!(observed(&d,8),InferredType::Values {classes,maybe_unset:false,..}
            if *classes==BTreeSet::from(["java.lang.String".into()])));
        assert!(matches!(observed(&d,9),InferredType::Values {classes,maybe_unset:false,..}
            if *classes==BTreeSet::from(["java.lang.Long".into(),"java.lang.String".into()])));
    }
    #[test]
    fn unpaired_or_invalid_start_cannot_supply_an_alternative_fact() {
        for bad in ["version","owner","capture","condition-kind"] {
            let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        # empty\n    else:\n        inspect {_value}");
            let initial=init(&mut f,1,&["java.lang.String"],true);
            let subject=read(&mut f,2);let c=condition(&mut f,2,subject,presence(false));
            let start=branch(&mut f,2,Some(c),&[]);
            for m in f.nodes[start as usize].metadata.iter_mut().filter(|m|m.key==FLOW) {
                if bad=="version" {m.value=json!({"version":2,"kind":"branch","conditionCapture":0,"bodyWhen":true}).to_string();}
                if bad=="owner" {m.owner_component_id=Some("another.provider".into());}
                if bad=="capture" {m.value=json!({"version":3,"kind":"branch","chain":"start","bodyWhen":true}).to_string();}
            }
            if bad=="condition-kind" {f.nodes[c as usize].kind=SyntaxKind::Expression;}
            let inside=inspect(&mut f,5);let alternate=branch(&mut f,4,None,&[inside]);
            let d=run(f,&[initial,start,alternate]);
            assert!(!present(&d,5),"{bad}: {:?}",observed(&d,5));
        }
    }
    #[test]
    fn effects_inside_else_use_the_current_binding_not_the_old_checked_value() {
        for operation in ["suspend","reassign","opaque"] {
            let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    if {_value} checked:\n        # empty\n    else:\n        bind {_value} = initial\n        inspect {_value}\n    inspect {_value}");
            let initial=init(&mut f,1,&["java.lang.String"],true);
            let subject=read(&mut f,2);let c=condition(&mut f,2,subject,presence(false));
            let start=branch(&mut f,2,Some(c),&[]);
            let effect=if operation=="reassign" {init(&mut f,5,&["java.lang.Long"],false)}else{
                let id=f.push(SyntaxKind::Effect,"effect",f.line(5));
                if operation=="suspend" {
                    f.nodes[id as usize].metadata.extend([
                        metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),
                        metadata(shared::BINDINGS,json!({"version":1,"local":"preserve","shared":"suspend"})),
                    ]);
                }
                id
            };
            let inside=inspect(&mut f,6);let alternate=branch(&mut f,4,None,&[effect,inside]);
            let after=inspect(&mut f,7);let d=run(f,&[initial,start,alternate,after]);
            if operation=="opaque" {assert!(matches!(observed(&d,6),InferredType::Unknown));}
            else {
                let class=if operation=="reassign" {"java.lang.Long"}else{"java.lang.String"};
                assert!(matches!(observed(&d,6),InferredType::Values {classes,maybe_unset:false,..}
                    if *classes==BTreeSet::from([class.to_owned()])));
            }
        }
    }
    pub(super) fn mixed_host()->(Arc<Catalog>,parser_wasm::ParserHost) {
        use sha2::{Digest,Sha256};
        use parser_wasm::host::{AddonDataPin,AddonDataSource};
        let snapshot=std::env::var("SKRIPT_REFLECT_SNAPSHOT").unwrap();
        let candidate=std::fs::read(std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").unwrap()).unwrap();
        let bytes=std::fs::read(std::env::var("SKRIPT_REFLECT_OWNED_DATA").unwrap()).unwrap();
        let envelope:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
        let data=AddonDataSource::from_json(AddonDataPin {
            component_id:"nlaocs.skript-reflect".into(),
            component_sha256:format!("{:x}",Sha256::digest(&candidate)),
            id:envelope["id"].as_str().unwrap().into(),schema_id:envelope["schemaId"].as_str().unwrap().into(),
            schema_version:envelope["schemaVersion"].as_u64().unwrap()as u32,
            sha256:format!("{:x}",Sha256::digest(&bytes)),
            bindings:serde_json::from_value(envelope["bindings"].clone()).unwrap(),
        },&bytes).unwrap();
        let catalog=Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),addon_data_sources:vec![data],
            runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()}).unwrap();
        host.load_addon(&candidate).unwrap();
        (catalog,host)
    }
    #[test]
    #[ignore="requires the explicit existing reflect snapshot/data and current candidate"]
    fn current_else_flow_mixed_provider_uses_only_current_header_and_branch_proofs() {
        let (catalog,mut host)=mixed_host();
        let source:Arc<str>=Arc::from("function mixedelse(value: text):\n    if {_value} is not set:\n        set {_copy} to 123\n    else:\n        set {_copy} to {_value}\n    set {_after} to {_copy}\n");
        let uri:Uri="file:///guards/mixed-else.sk".parse().unwrap();
        let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///guards",uri.as_str(),1,source.as_ref()),Default::default()).unwrap();
        assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
        assert!(result.syntax.diagnostics.is_empty(),"{:?}",result.syntax.diagnostics);
        let d=Document::from_result_with_host(&result,source,uri,catalog,&mut host,None).unwrap();
        let hints=d.write_type_hints();
        assert!(hints.iter().any(|h|h.position.line==4&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s==": string")),"{hints:?}");
        assert!(hints.iter().any(|h|h.position.line==5&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s.contains("long")&&s.contains("string")&&!s.contains('?'))),"{hints:?}");
    }
    #[test]
    fn current_else_flow_core_component_and_lsp_follow_assignment_and_nullable_edges() {
        let catalog=Arc::new(ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain")).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),
            runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()}).unwrap();
        let uri:Uri="file:///guards/else.sk".parse().unwrap();
        let body="function branchcopy(value: text):\n    if {_value} is not set:\n        set {_copy} to 123\n    else:\n        set {_copy} to {_value}\n    set {_after} to {_copy}\n";
        for (revision,prefix,has_else,reopen) in [
            (1,"",true,false),(2,"# Unicode \u{1f980}\r\n",true,false),
            (4,"",false,false),(0,"",true,true),
        ] {
            if revision==4 {
                let token=parser_wasm::DocumentCancellationToken::new();token.cancel();
                let cancelled=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///guards",uri.as_str(),3,body),
                    parser_wasm::DocumentParserConfig {cancellation:token,..Default::default()});
                assert!(matches!(cancelled,Err(parser_wasm::DocumentParseError::Cancelled {..})));
            }
            if reopen {assert!(host.release_document("file:///guards",uri.as_str(),4).unwrap());}
            let body=if has_else {body.to_owned()}else{body.replace("    else:\n        set {_copy} to {_value}\n","")};
            let source:Arc<str>=format!("{prefix}{body}").into();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///guards",uri.as_str(),revision,source.as_ref()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            assert!(result.syntax.diagnostics.is_empty(),"{:?}",result.syntax.diagnostics);
            let d=Document::from_result_with_host(&result,source,uri.clone(),catalog.clone(),&mut host,None).unwrap();
            let hints=d.write_type_hints();let delta=u32::from(!prefix.is_empty());
            if has_else {
                assert!(hints.iter().any(|h|h.position.line==4+delta&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s==": string")),"{hints:?}");
                assert!(hints.iter().any(|h|h.position.line==5+delta&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s.contains("long")&&s.contains("string")&&!s.contains('?'))),"{hints:?}");
            }else{
                let at=d.source.rfind("{_copy}").unwrap()+3;
                let at=d.lines.position(&d.source,at).unwrap();
                assert!(matches!(d.type_at(at).unwrap().1,InferredType::Unknown),"deleted else must not initialize the bypass");
            }
            assert_eq!(result.ast.nodes.iter().filter(|n|unique_metadata(n,FLOW).is_some_and(|j|j.contains("\"version\":4"))).count(),if has_else {2}else{1});
        }
    }
}


#[cfg(test)]
mod elseif_flow {
    use super::*;
    use super::else_flow::{branch,run,present};
    fn chain(f:&mut Fixture,row:usize,condition:Option<u64>,body:&[u64],role:&str)->u64 {
        let id=branch(f,row,condition,body);
        let mut flow=json!({"version":4,"kind":"branch","chain":role});
        if condition.is_some(){flow["conditionCapture"]=json!(0);flow["bodyWhen"]=json!(true);}
        f.nodes[id as usize].metadata.iter_mut().find(|m|m.key==FLOW).unwrap().value=flow.to_string();id
    }
    fn inspect_name(f:&mut Fixture,row:usize,spelling:&str,name:&str)->u64 {
        let subject=read_named(f,row,spelling,name);let id=f.push(SyntaxKind::Effect,"effect",f.line(row));f.seal(id,&[(0,subject)]);
        f.nodes[id as usize].metadata.extend([metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),
            metadata(OPERANDS,json!({"version":1,"prefix":[0]})),metadata(shared::BINDINGS,json!({"version":1,"local":"preserve","shared":"preserve"}))]);id
    }
    #[test]
    fn continued_false_facts_are_independent_of_taken_assignments_suspension_and_opaque_effects() {
        for operation in ["preserve","reassign","suspend","opaque"] {
            let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    bind {_other} = initial\n    if {_value} checked:\n        bind {_value} = initial\n    else if {_other} checked:\n        inspect {_value}\n    else:\n        inspect {_value}\n    inspect {_value}");
            let value=init(&mut f,1,&["java.lang.String"],true);
            let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);
            let subject=read(&mut f,3);let c=condition(&mut f,3,subject,presence(false));
            let effect=if operation=="reassign" {init(&mut f,4,&["java.lang.Long"],false)}else{
                let id=f.push(SyntaxKind::Effect,"effect",f.line(4));
                if operation!="opaque" {f.nodes[id as usize].metadata.extend([
                    metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})),
                    metadata(shared::BINDINGS,json!({"version":1,"local":"preserve","shared":if operation=="suspend"{"suspend"}else{"preserve"}})),
                ]);}id
            };
            let first=chain(&mut f,3,Some(c),&[effect],"start");
            let subject=read_named(&mut f,5,"{_other}","other");let c=condition(&mut f,5,subject,presence(false));
            let inside=inspect(&mut f,6);let next=chain(&mut f,5,Some(c),&[inside],"next");
            let inside=inspect(&mut f,8);let last=chain(&mut f,7,None,&[inside],"else");
            let after=inspect(&mut f,9);let d=run(f,&[value,other,first,next,last,after]);
            for row in [6,8] {assert!(matches!(observed(&d,row),InferredType::Values {classes,maybe_unset:false,..}
                if *classes==BTreeSet::from(["java.lang.String".into()])),"{operation}: {row}: {:?}",observed(&d,row));}
            if operation=="opaque" {assert!(matches!(observed(&d,9),InferredType::Unknown));}
            if operation=="reassign" {assert!(matches!(observed(&d,9),InferredType::Values {classes,maybe_unset:false,..}
                if *classes==BTreeSet::from(["java.lang.Long".into(),"java.lang.String".into()])));}
        }
    }
    #[test]
    fn every_taken_exit_and_the_unconsumed_false_path_reach_the_join() {
        for with_else in [false,true] {
            let ending=if with_else {"    else:\n        bind {_result} = initial\n    inspect {_result}"}else{"    inspect {_result}"};
            let source=format!("owner:\n    bind {{_value}} = initial\n    bind {{_other}} = initial\n    bind {{_result}} = initial\n    if {{_value}} checked:\n        bind {{_result}} = initial\n    else if {{_other}} checked:\n        bind {{_result}} = initial\n{ending}");
            let mut f=Fixture::new(&source);let value=init(&mut f,1,&["java.lang.String"],true);
            let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);
            let result=init_named(&mut f,3,"{_result}","result",&["java.lang.Boolean"],false);
            let subject=read(&mut f,4);let c=condition(&mut f,4,subject,presence(false));
            let left=init_named(&mut f,5,"{_result}","result",&["java.lang.Long"],false);let first=chain(&mut f,4,Some(c),&[left],"start");
            let subject=read_named(&mut f,6,"{_other}","other");let c=condition(&mut f,6,subject,presence(false));
            let middle=init_named(&mut f,7,"{_result}","result",&["java.lang.String"],false);let next=chain(&mut f,6,Some(c),&[middle],"next");
            let mut ids=vec![value,other,result,first,next];
            if with_else {let right=init_named(&mut f,9,"{_result}","result",&["java.lang.Double"],false);ids.push(chain(&mut f,8,None,&[right],"else"));}
            let row=if with_else {10}else{8};ids.push(inspect_name(&mut f,row,"{_result}","result"));let d=run(f,&ids);
            let expected=BTreeSet::from(["java.lang.Long".into(),"java.lang.String".into(),if with_else{"java.lang.Double".into()}else{"java.lang.Boolean".into()}]);
            assert!(matches!(observed(&d,row as u32),InferredType::Values {classes,maybe_unset:false,..}if *classes==expected),"{with_else}: {:?}",observed(&d,row as u32));
        }
    }
    #[test]
    fn a_third_condition_does_not_drop_earlier_taken_exits_or_the_last_false_exit() {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    bind {_other} = initial\n    bind {_third} = initial\n    bind {_result} = initial\n    if {_value} checked:\n        bind {_result} = initial\n    else if {_other} checked:\n        bind {_result} = initial\n    else if {_third} checked:\n        bind {_result} = initial\n    inspect {_result}");
        let value=init(&mut f,1,&["java.lang.String"],true);let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);let third=init_named(&mut f,3,"{_third}","third",&["java.lang.String"],true);let result=init_named(&mut f,4,"{_result}","result",&["java.lang.Boolean"],false);
        let subject=read(&mut f,5);let c=condition(&mut f,5,subject,presence(false));let assigned=init_named(&mut f,6,"{_result}","result",&["java.lang.Long"],false);let first=chain(&mut f,5,Some(c),&[assigned],"start");
        let subject=read_named(&mut f,7,"{_other}","other");let c=condition(&mut f,7,subject,presence(false));let assigned=init_named(&mut f,8,"{_result}","result",&["java.lang.String"],false);let next=chain(&mut f,7,Some(c),&[assigned],"next");
        let subject=read_named(&mut f,9,"{_third}","third");let c=condition(&mut f,9,subject,presence(false));let assigned=init_named(&mut f,10,"{_result}","result",&["java.lang.Double"],false);let last=chain(&mut f,9,Some(c),&[assigned],"next");let after=inspect_name(&mut f,11,"{_result}","result");
        let d=run(f,&[value,other,third,result,first,next,last,after]);let expected=BTreeSet::from(["java.lang.Boolean".into(),"java.lang.Long".into(),"java.lang.String".into(),"java.lang.Double".into()]);
        assert!(matches!(observed(&d,11),InferredType::Values {classes,maybe_unset:false,..}if *classes==expected));
    }
    #[test]
    fn nested_outcomes_in_a_continuation_do_not_replace_the_outer_false_path() {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    bind {_other} = initial\n    if {_value} checked:\n        bind {_value} = initial\n    else if {_other} checked:\n        if {_other} checked:\n            bind {_value} = initial\n        else:\n            bind {_value} = initial\n    else:\n        inspect {_value}\n    inspect {_value}");
        let value=init(&mut f,1,&["java.lang.String"],true);let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);
        let subject=read(&mut f,3);let c=condition(&mut f,3,subject,presence(false));let assigned=init(&mut f,4,&["java.lang.Long"],false);let first=chain(&mut f,3,Some(c),&[assigned],"start");
        let subject=read_named(&mut f,6,"{_other}","other");let c=condition(&mut f,6,subject,presence(true));let assigned=init(&mut f,7,&["java.lang.Double"],false);let inner=chain(&mut f,6,Some(c),&[assigned],"start");let assigned=init(&mut f,9,&["java.lang.Boolean"],false);let inner_else=chain(&mut f,8,None,&[assigned],"else");
        let subject=read_named(&mut f,5,"{_other}","other");let c=condition(&mut f,5,subject,presence(false));let next=chain(&mut f,5,Some(c),&[inner,inner_else],"next");
        let inside=inspect(&mut f,11);let last=chain(&mut f,10,None,&[inside],"else");let after=inspect(&mut f,12);let d=run(f,&[value,other,first,next,last,after]);
        assert!(matches!(observed(&d,11),InferredType::Values {classes,maybe_unset:false,..}if *classes==BTreeSet::from(["java.lang.String".into()])));
        let expected=BTreeSet::from(["java.lang.Long".into(),"java.lang.Double".into(),"java.lang.Boolean".into(),"java.lang.String".into()]);assert!(matches!(observed(&d,12),InferredType::Values {classes,maybe_unset:false,..}if *classes==expected));
    }
    #[test]
    fn invalid_or_unpaired_continuations_do_not_consume_or_create_a_chain() {
        for bad in ["owner","old-role","mixed-version","capture","kind","operand","barrier","unpaired"] {
            let gap=if bad=="barrier" {"unparsed statement"}else{"# gap"};
            let source=format!("owner:\n    bind {{_value}} = initial\n    bind {{_other}} = initial\n    if {{_value}} checked:\n        # empty\n    {gap}\n    else if {{_other}} checked:\n        inspect {{_value}}\n    else:\n        inspect {{_value}}");
            let mut f=Fixture::new(&source);let value=init(&mut f,1,&["java.lang.String"],true);
            let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);
            let subject=read(&mut f,3);let c=condition(&mut f,3,subject,presence(false));let first=chain(&mut f,3,Some(c),&[],if bad=="unpaired"{"next"}else{"start"});
            let subject=read_named(&mut f,6,"{_other}","other");let c=condition(&mut f,6,subject,presence(false));
            let inside=inspect(&mut f,7);let next=chain(&mut f,6,Some(c),&[inside],"next");
            let metadata=f.nodes[next as usize].metadata.iter_mut().find(|m|m.key==FLOW).unwrap();
            if bad=="owner" {metadata.owner_component_id=Some("another.provider".into());}
            if bad=="old-role" {metadata.value=json!({"version":3,"kind":"branch","chain":"next","conditionCapture":0,"bodyWhen":true}).to_string();}
            if bad=="capture" {metadata.value=json!({"version":4,"kind":"branch","chain":"next","conditionCapture":99,"bodyWhen":true}).to_string();}
            if bad=="mixed-version" {f.nodes[first as usize].metadata.iter_mut().find(|m|m.key==FLOW).unwrap().value=json!({"version":3,"kind":"branch","chain":"start","conditionCapture":0,"bodyWhen":true}).to_string();}
            if bad=="kind" {f.nodes[c as usize].kind=SyntaxKind::Expression;}
            if bad=="operand" {let extra=f.push(SyntaxKind::Expression,"literal",f.part(6,"checked"));f.nodes[next as usize].children.push(extra);}
            let inside=inspect(&mut f,9);let last=chain(&mut f,8,None,&[inside],"else");let d=run(f,&[value,other,first,next,last]);
            for row in [7,9] {assert!(!present(&d,row),"{bad}: {row}: {:?}",observed(&d,row));}
        }
    }
    #[test]
    fn parser_context_is_not_restored_from_a_runtime_false_path() {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    bind {_other} = initial\n    if {_value} checked:\n        opaque operation\n    else if {_other} checked:\n        inspect {_value}");
        let value=init(&mut f,1,&["java.lang.String"],true);let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);
        let subject=read(&mut f,3);let c=condition(&mut f,3,subject,presence(false));let effect=f.push(SyntaxKind::Effect,"effect",f.line(4));let first=chain(&mut f,3,Some(c),&[effect],"start");
        let subject=read_named(&mut f,5,"{_other}","other");let c=condition(&mut f,5,subject,presence(false));let inside=inspect(&mut f,6);let next=chain(&mut f,5,Some(c),&[inside],"next");
        f.nodes[next as usize].metadata.iter_mut().find(|m|m.key==OPERANDS).unwrap().value=json!({"version":1,"prefix":[0],"requiredContext":{"key":"entry","value":"current"}}).to_string();
        let root=f.container(0,SyntaxKind::Structure,&[value,other,first,next],None);
        f.nodes[root as usize].metadata.push(metadata(EVALUATION_CONTEXT,json!({"version":1,"kind":"scope","key":"entry","value":"current"})));
        let d=finish(f,root,&mut Reads {mode:"identity",latest:None,calls:Vec::new()});assert!(matches!(observed(&d,6),InferredType::Unknown));
    }

    #[test]
    fn binding_effects_without_a_target_only_contract_do_not_keep_other_facts() {
        for target_only in [false,true] {
            let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    bind {_other} = initial\n    inspect {_value}");let value=init(&mut f,1,&["java.lang.String"],false);let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],false);
            let entry=f.nodes[other as usize].metadata.iter_mut().find(|m|m.key==CHANGE).unwrap();let mut contract:serde_json::Value=serde_json::from_str(&entry.value).unwrap();contract["targetOnly"]=json!(target_only);entry.value=contract.to_string();
            let after=inspect(&mut f,3);let d=run(f,&[value,other,after]);if target_only {assert!(present(&d,3));}else{assert!(matches!(observed(&d,3),InferredType::Unknown));}
        }
    }
    #[test]
    fn a_taken_raw_barrier_expires_parser_context_without_restoring_it_on_false_input() {
        for barrier in [false,true] {
            let text=if barrier {"unparsed statement"}else{"# empty body"};
            let source=format!("owner:\n    bind {{_value}} = initial\n    bind {{_other}} = initial\n    if {{_value}} checked:\n        {text}\n    else if {{_other}} checked:\n        inspect {{_value}}");
            let mut f=Fixture::new(&source);let value=init(&mut f,1,&["java.lang.String"],true);let other=init_named(&mut f,2,"{_other}","other",&["java.lang.String"],true);
            let subject=read(&mut f,3);let c=condition(&mut f,3,subject,presence(false));let first=chain(&mut f,3,Some(c),&[],"start");
            let subject=read_named(&mut f,5,"{_other}","other");let c=condition(&mut f,5,subject,presence(false));let inside=inspect(&mut f,6);let next=chain(&mut f,5,Some(c),&[inside],"next");
            f.nodes[next as usize].metadata.iter_mut().find(|m|m.key==OPERANDS).unwrap().value=json!({"version":1,"prefix":[0],"requiredContext":{"key":"entry","value":"current"}}).to_string();
            // Every published node preserves parser/init context. Only the
            // missing raw statement inside the first taken body can expire it.
            for n in &mut f.nodes {n.metadata.push(metadata(EVALUATION_CONTEXT,json!({"version":1,"kind":"preserve"})));}
            let root=f.container(0,SyntaxKind::Structure,&[value,other,first,next],None);
            f.nodes[root as usize].metadata.push(metadata(EVALUATION_CONTEXT,json!({"version":1,"kind":"scope","key":"entry","value":"current"})));
            let d=finish(f,root,&mut Reads {mode:"identity",latest:None,calls:Vec::new()});
            if barrier {assert!(matches!(observed(&d,6),InferredType::Unknown));}else{assert!(present(&d,6));}
        }
    }

    fn exercise_current_elseif(host:&mut parser_wasm::ParserHost,catalog:Arc<Catalog>,suffix:&str) {
        let uri:Uri=format!("file:///guards/{suffix}.sk").parse().unwrap();
        let body="function chaincopy(value: text, other: text):\n    if {_value} is not set:\n        set {_result} to 123\n    else if {_other} is not set:\n        set {_result} to {_value}\n    else:\n        set {_result} to {_other}\n    set {_after} to {_result}\n";
        for (revision,prefix,has_else,reopen) in [(1,"",true,false),(2,"# Unicode \u{1f980}\r\n",true,false),(4,"",false,false),(0,"",true,true)] {
            if revision==4 {let token=parser_wasm::DocumentCancellationToken::new();token.cancel();let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///guards",uri.as_str(),3,body),parser_wasm::DocumentParserConfig {cancellation:token,..Default::default()});assert!(matches!(result,Err(parser_wasm::DocumentParseError::Cancelled {..})));}
            if reopen {assert!(host.release_document("file:///guards",uri.as_str(),4).unwrap());}
            let body=if has_else {body.to_owned()}else{body.replace("    else:\n        set {_result} to {_other}\n","")};let source:Arc<str>=format!("{prefix}{body}").into();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///guards",uri.as_str(),revision,source.as_ref()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);assert!(result.syntax.diagnostics.is_empty(),"{:?}",result.syntax.diagnostics);
            let d=Arc::new(Document::from_result_with_host(&result,source,uri.clone(),catalog.clone(),host,None).unwrap());let delta=u32::from(!prefix.is_empty());let hints=d.write_type_hints();
            assert!(hints.iter().any(|h|h.position.line==4+delta&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s==": string")),"{hints:?}");
            if has_else {assert!(hints.iter().any(|h|h.position.line==6+delta&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s==": string")),"{hints:?}");assert!(hints.iter().any(|h|h.position.line==7+delta&&matches!(&h.label,lsp_types::InlayHintLabel::String(s)if s.contains("long")&&s.contains("string")&&!s.contains('?'))),"{hints:?}");}
            else {let at=d.source.rfind("{_result}").unwrap()+3;assert!(matches!(d.type_at(d.lines.position(&d.source,at).unwrap()).unwrap().1,InferredType::Unknown),"missing else must retain uninitialized false exit");}
            assert_eq!(result.ast.nodes.iter().filter(|n|unique_metadata(n,FLOW).is_some_and(|j|j.contains("\"version\":4"))).count(),if has_else {3}else{2});
            let start=d.source.rfind("{_value}").unwrap();let edit=TextRange::new(start,start+"{_value}".len());let position=d.lines.position(&d.source,start+3).unwrap();
            let (_,fields)=d.hover_fields(position).unwrap();assert!(fields.iter().any(|(key,value)|*key=="Current type"&&value=="string"),"{fields:?}");
            let (ty,_,_)=d.completion_input(&d.source,edit,"value").expect("current adopted operand completion boundary");assert!(matches!(ty,InferredType::Values {maybe_unset:false,classes,..}if *classes==BTreeSet::from(["java.lang.String".into()])));
            let mut index=crate::lsp::completion::CatalogIndex::with_documentation_format(&catalog,skript_parser::RawTreeOptions::for_skript_version(2,15),lsp_types::MarkupKind::PlainText);index.certify_types(host);
            let mut view=crate::lsp::completion::Document::from_result(&result,Arc::clone(&d.source),Arc::new(index)).unwrap();view.semantic_types=Some(Arc::clone(&d));
            let cursor=d.lines.position(&d.source,start+"{_val".len()).unwrap();let list=view.at(cursor,false).unwrap();let item=list.items.iter().find(|i|i.label=="{_value}").expect("the current parameter candidate");
            let Some(lsp_types::CompletionTextEdit::Edit(insert))=&item.text_edit else {panic!("variable candidate requires a plain current edit")};
            let from=d.lines.offset(&d.source,insert.range.start).unwrap();let to=d.lines.offset(&d.source,insert.range.end).unwrap();let mut accepted=d.source.to_string();accepted.replace_range(from..to,&insert.new_text);assert_eq!(accepted,d.source.as_ref(),"completion must preserve the existing suffix and braces");

        }
    }
    #[test]
    fn current_elseif_flow_core_component_and_lsp_follow_false_edges_edits_and_reopen() {
        let catalog=Arc::new(ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain")).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()}).unwrap();
        exercise_current_elseif(&mut host,catalog,"core-elseif");
    }
    #[test]
    #[ignore="requires the explicit existing reflect snapshot/data and current candidate"]
    fn current_elseif_flow_mixed_provider_keeps_fresh_branch_proofs() {
        let (catalog,mut host)=super::else_flow::mixed_host();exercise_current_elseif(&mut host,catalog,"mixed-elseif");
    }
}
