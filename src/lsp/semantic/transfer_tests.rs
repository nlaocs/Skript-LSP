use super::*;
use crate::lsp::semantic::transfer::{BindingEffects,Output,SourceUpdate,TypeBound,Resolver};

fn bound(class:Option<&str>,nullable:bool)->TypeBound {
    TypeBound {types:class.into_iter().map(str::to_owned).collect(),complete:true,multiple:false,maybe_unset:nullable}
}
fn attach(f:&mut Fixture,id:u64,read:bool) {
    let n=&mut f.nodes[id as usize];
    n.metadata.retain(|e|e.key!=VARIABLE_READ);
    if read {n.metadata.push(metadata(VARIABLE_READ,json!({"version":3,"requestedTypes":["java.lang.Object"],"mayRefreshPlayer":true,"noRefreshTypes":[],"missingValue":"empty"})));}
    n.metadata.push(metadata(parser_wasm::host::semantic_transfer::METADATA,json!({"version":1,"operation":"fixture.transfer","parameters":"{}"})));
}
fn copy(f:&mut Fixture,row:usize,target:&str,source:&str)->(u64,u64) {
    let value=f.var(row,source,&source[2..source.len()-1],true,false);attach(f,value,true);
    let target=f.var(row,target,&target[2..target.len()-1],true,false);
    let effect=f.push(SyntaxKind::Effect,"effect",f.line(row));
    f.seal(effect,&[(0,target),(1,value)]);
    f.nodes[effect as usize].metadata.extend([
        metadata(CHANGE,json!({"version":6,"mode":"set","bindingRole":"binding","targetCapture":0,"valueCapture":1,"targetOnly":false})),
        metadata(OPERANDS,json!({"version":1,"prefix":[1]})),
    ]);
    attach(f,effect,false);(effect,value)
}
fn finish(f:Fixture,root:u64,resolver:&mut dyn Resolver)->Document {
    let mapped=skript_parser::MappedSource::identity(Arc::clone(&f.source));
    let raw=skript_parser::parse_raw_tree(&mapped,skript_parser::RawTreeOptions::for_skript_version(2,15));
    Document::from_ast(f.source,"file:///transfer.sk".parse().unwrap(),catalog(),&AstTree {roots:vec![root],nodes:f.nodes},&raw,
        Vec::new(),Vec::new(),None,None,HashSet::new(),Vec::new(),Some(resolver)).unwrap()
}
#[test]
fn returned_type_source_poststate_other_effects_and_destination_are_separate() {
    for (invalidate,source_known) in [(false,true),(true,false),(true,true)] {
      for inspect_source in [false,true] {
        // Each observation is a separate execution: an opaque inspection can
        // itself invalidate facts, so do not inspect one binding before another.
        let text=format!("owner:\n    bind {{_old}} = 1\n    bind {{_source}} = initial\n    bind {{_copy}} = {{_source}}\n    inspect {{_{}}}",if inspect_source {"source"}else{"old"});
        let mut f=Fixture::new(&text);
        let old=f.assign(1,"{_old}","old","1","java.lang.Long",true,false,"set");
        let source=f.assign(2,"{_source}","source","initial","unknown.Before",true,false,"set");
        let (copied,read)=copy(&mut f,3,"{_copy}","{_source}");
        let observed=if inspect_source {f.read(4,"{_source}","source",true,false)}else{f.read(4,"{_old}","old",true,false)};
        let root=f.container(0,SyntaxKind::Structure,&[old,source,copied,observed],None);
        let mut calls=Vec::new();
        let mut resolver=|id,input:TypeBound|{
            calls.push(id);
            if id==read {
                assert_eq!(input.types,["unknown.Before"]);
                Ok(Some(Output {value:bound(Some("unknown.Returned"),true),source:if source_known {SourceUpdate::Known(bound(Some("unknown.SourceAfter"),true))}else{SourceUpdate::Unknown},
                    source_writeback:Some(bound(Some("unknown.BranchWrite"),true)),effects:if invalidate {BindingEffects::Invalidate}else{BindingEffects::Preserve}}))
            }else{
                assert_eq!(id,copied);assert_eq!(input.types,["unknown.Returned"]);assert!(input.maybe_unset);
                Ok(Some(Output {value:bound(Some("unknown.Stored"),true),source:SourceUpdate::Unchanged,source_writeback:None,effects:BindingEffects::Preserve}))
            }
        };
        let d=finish(f,root,&mut resolver);
        assert_eq!(calls,[read,copied]);
        assert_eq!(d.writes[2].post_type.proven_types().unwrap().0,&BTreeSet::from(["unknown.Stored".into()]));
        let observed=d.occurrences.iter().find(|o|o.range.start==d.source.find("inspect ").unwrap()+8).unwrap();
        if !inspect_source {assert_eq!(matches!(observed.ty,InferredType::Unknown),invalidate);}
        else if source_known {
            assert_eq!(observed.ty.proven_types().unwrap().0,&BTreeSet::from(["unknown.SourceAfter".into()]));
            assert!(matches!(&observed.reference,Reference::Variable {reaching,..} if reaching.is_empty()));
        } else {assert!(matches!(observed.ty,InferredType::Unknown));}
      }
    }
}
#[test]
fn self_copy_applies_final_set_after_read_effects() {
    let mut f=Fixture::new("owner:\n    bind {_source} = initial\n    bind {_source} = {_source}");
    let initial=f.assign(1,"{_source}","source","initial","unknown.Before",true,false,"set");
    let (set,read)=copy(&mut f,2,"{_source}","{_source}");
    let root=f.container(0,SyntaxKind::Structure,&[initial,set],None);
    let mut resolver=|id,input:TypeBound|Ok(Some(if id==read {
        assert_eq!(input.types,["unknown.Before"]);
        Output {value:bound(Some("unknown.Returned"),true),source:SourceUpdate::Unknown,source_writeback:None,effects:BindingEffects::Invalidate}
    }else{
        assert_eq!(id,set);assert_eq!(input.types,["unknown.Returned"]);
        Output {value:bound(Some("unknown.Final"),true),source:SourceUpdate::Unknown,source_writeback:None,effects:BindingEffects::Invalidate}
    }));
    let d=finish(f,root,&mut resolver);
    assert_eq!(d.writes.last().unwrap().post_type.proven_types().unwrap().0,&BTreeSet::from(["unknown.Final".into()]));
}
#[test]
fn known_empty_read_deletes_an_existing_destination_instead_of_restoring_it() {
    let mut f=Fixture::new("owner:\n    bind {_source} = empty\n    bind {_target} = 1\n    bind {_target} = {_source}");
    let empty=f.assign(1,"{_source}","source","empty","java.lang.String",true,false,"delete");
    let target=f.assign(2,"{_target}","target","1","java.lang.Long",true,false,"set");
    let (set,read)=copy(&mut f,3,"{_target}","{_source}");
    let root=f.container(0,SyntaxKind::Structure,&[empty,target,set],None);
    let mut calls=Vec::new();
    let mut resolver=|id,input:TypeBound|{calls.push(id);assert!(input.types.is_empty()&&input.maybe_unset);
        Ok(Some(Output {value:bound(None,true),source:SourceUpdate::Unchanged,source_writeback:None,effects:BindingEffects::Preserve}))};
    let d=finish(f,root,&mut resolver);assert_eq!(calls,[read,set]);
    assert!(matches!(&d.writes.last().unwrap().post_type,InferredType::Values {classes,maybe_unset:true,..}if classes.is_empty()));
}
