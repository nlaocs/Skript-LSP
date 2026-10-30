use super::*;
fn effects(f:&mut Fixture,id:u64,shared:&str) {
    f.nodes[id as usize].metadata.push(metadata(crate::lsp::semantic::shared::BINDINGS,json!({"version":1,"local":"preserve","shared":shared})));
}
fn initialize_global(f:&mut Fixture,row:usize)->u64 {
    let id=f.assign(row,"{shared}","shared","initial","java.lang.String",false,false,"set");
    f.nodes[id as usize].metadata.retain(|m|m.key!=CHANGE);
    f.nodes[id as usize].metadata.push(metadata(CHANGE,json!({"version":6,"bindingRole":"binding","mode":"set","targetCapture":0,"valueCapture":1,"targetOnly":false})));
    f.nodes[id as usize].metadata.push(metadata(parser_wasm::host::semantic_transfer::METADATA,json!({"version":2,"outputUse":"flow","operation":"fixture.global-set","parameters":"{}"})));
    id
}
#[test]
fn current_global_requires_an_explicit_interference_contract_and_expires_at_generic_barriers() {
    use crate::lsp::semantic::transfer::{Output,SourceUpdate,BindingEffects};
    for (isolated,barrier,usage) in [(false,"preserve","flow"),(true,"preserve","flow"),(true,"suspend","flow"),(true,"reenter","flow"),(true,"invalidate","flow"),(true,"preserve","completion-candidate")] {
        let mut f=Fixture::new("owner:\n    bind {shared} = initial\n    pause\n    inspect {shared}");
        let set=initialize_global(&mut f,1);
        let op=f.nodes[set as usize].metadata.iter_mut().find(|m|m.key==parser_wasm::host::semantic_transfer::METADATA).unwrap();
        let mut value:serde_json::Value=serde_json::from_str(&op.value).unwrap();value["outputUse"]=usage.into();op.value=value.to_string();
        let pause=f.push(SyntaxKind::Effect,"effect",f.line(2));
        f.nodes[pause as usize].metadata.push(metadata(OPERANDS,json!({"version":1,"prefix":[],"complete":true})));
        let read=f.read(3,"{shared}","shared",false,false);
        let root=f.container(0,SyntaxKind::Structure,&[set,pause,read],None);
        for id in 0..f.nodes.len() as u64 {effects(&mut f,id,"preserve");}
        f.nodes[pause as usize].metadata.retain(|m|m.key!=crate::lsp::semantic::shared::BINDINGS);
        effects(&mut f,pause,barrier);
        if isolated {f.nodes[root as usize].metadata.push(metadata(crate::lsp::semantic::shared::SCOPE,json!({"version":1,"model":"serialized-segment","interference":"excluded-by-provider-contract"})));}
        let source=Arc::clone(&f.source);
        let mapped=skript_parser::MappedSource::identity(Arc::clone(&source));
        let raw=skript_parser::parse_raw_tree(&mapped,skript_parser::RawTreeOptions::for_skript_version(2,15));
        let mut resolver=|id,input:crate::lsp::semantic::transfer::TypeBound|{assert_eq!(id,set);Ok(Some(Output {value:input,source:SourceUpdate::Unchanged,source_writeback:None,effects:BindingEffects::Preserve}))};
        let d=Document::from_ast(source,"file:///shared.sk".parse().unwrap(),catalog(),&AstTree {roots:vec![root],nodes:f.nodes},&raw,
            Vec::new(),Vec::new(),None,None,HashSet::new(),Vec::new(),Some(&mut resolver)).unwrap();
        let position=Position::new(3,14);
        assert_eq!(d.type_at(position).unwrap().1.proven_types().is_some(),isolated&&barrier=="preserve"&&usage=="flow","{isolated}/{barrier}/{usage}");
    }
}
#[test]
fn an_addon_defined_suspension_preserves_local_binding_types_without_a_wait_name_test() {
    for local in ["preserve","invalidate"] {
    let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    yield-control\n    inspect {_value}");
    let set=f.assign(1,"{_value}","value","initial","java.lang.String",true,false,"set");
    let pause=f.push(SyntaxKind::Effect,"addon-defined-yield",f.line(2));
    f.nodes[pause as usize].metadata.push(metadata(OPERANDS,json!({"version":1,"prefix":[],"complete":true})));
    f.nodes[pause as usize].metadata.push(metadata(crate::lsp::semantic::shared::BINDINGS,json!({"version":1,"local":local,"shared":"suspend"})));
    let read=f.read(3,"{_value}","value",true,false);
    let root=f.container(0,SyntaxKind::Structure,&[set,pause,read],None);
    let d=f.finish(&[root]);assert_eq!(d.type_at(Position::new(3,14)).unwrap().1.proven_types().is_some(),local=="preserve");
    }
}
#[test]
fn suspension_evaluates_operand_effects_before_preserving_local_bindings() {
    for (pure_operand,invalid_complete) in [(false,false),(true,false),(true,true)] {
        let mut f=Fixture::new("owner:\n    bind {_value} = initial\n    yield-control 1 tick\n    inspect {_value}");
        let set=f.assign(1,"{_value}","value","initial","java.lang.String",true,false,"set");
        let duration=f.push(SyntaxKind::Expression,"literal",f.part(2,"1 tick"));
        if pure_operand {f.nodes[duration as usize].metadata.push(metadata(LOCAL_EFFECTS,json!({"version":1,"kind":"preserve"})));}
        let pause=f.push(SyntaxKind::Effect,"addon-defined-yield",f.line(2));
        f.seal(pause,&[(0,duration)]);
        f.nodes[pause as usize].metadata.push(metadata(OPERANDS,json!({"version":1,"prefix":[0],"complete":invalid_complete})));
        effects(&mut f,pause,"suspend");
        let read=f.read(3,"{_value}","value",true,false);
        let root=f.container(0,SyntaxKind::Structure,&[set,pause,read],None);
        let d=f.finish(&[root]);
        assert_eq!(d.type_at(Position::new(3,14)).unwrap().1.proven_types().is_some(),pure_operand&&!invalid_complete);
    }
}
#[test]
fn foreign_and_duplicate_binding_contracts_do_not_become_harmless_effects() {
    for bad in ["foreign","duplicate","version","unknown"] {
        let mut f=Fixture::new("opaque");let id=f.push(SyntaxKind::Effect,"addon-effect",TextRange::new(0,6));
        effects(&mut f,id,"suspend");
        let node=&mut f.nodes[id as usize];
        match bad {
            "foreign"=>node.metadata[0].owner_component_id=Some("foreign".into()),
            "duplicate"=>node.metadata.push(node.metadata[0].clone()),
            "version"=>node.metadata[0].value=json!({"version":2,"local":"preserve","shared":"suspend"}).to_string(),
            _=>node.metadata[0].value=json!({"version":1,"local":"preserve","shared":"unknown"}).to_string(),
        }
        retain_semantic_contracts(node,Some("fixture.semantic-provider"));
        assert!(!crate::lsp::semantic::shared::local_preserved(node),"{bad}");
    }
}

