use super::*;

fn preparation(source:&MappedSource,node:&RawNode,id:u64,parent:u64)->crate::ReusedSectionPreparation {
    let range=structure_header_range(source,node).unwrap();
    crate::ReusedSectionPreparation {attempt_id:id,parent_attempt_id:Some(parent),candidate:crate::SectionCandidate {
        raw_node_id:node.id,matched:CandidateMatch {extension_state:None,kind:crate::MatchSyntaxKind::Section,
            definition_id:"fixture.section".into(),registration_id:"fixture.section#0".into(),priority:0,registration_order:0,literal_anchor:false,pattern_index:0,pattern:node.text.clone(),
            matched:crate::PatternMatch {span:MatchSpan {local_range:TextRange::new(0,range.len()),mapped:source.map_range(range).unwrap()},captures:Vec::new(),tags:Vec::new(),mark:0,marks:Vec::new(),recovered_failures:Vec::new()}},
        element_class:None,addon_name:None,addon_version:None,owner_component_id:None,parsed_captures:Vec::new(),loop_section:false,effect_section:false,section_expression:false,body_mode:crate::SectionBodyMode::Trigger,
        body:Vec::new(),handler:None,metadata:BTreeMap::new(),body_context:ExpressionParseContext::default(),}}
}

#[test]
fn body_reuse_preparation_rejects_sibling_as_parent_and_changed_header_geometry() {
    let source=MappedSource::identity("function f():\n    if x:\n        if y:\n            # inner\n    if z:\n        # sibling\n");
    let tree=crate::parse_raw_tree(&source,crate::RawTreeOptions::for_skript_version(2,15));
    let root=tree.get(tree.roots[0]).unwrap();
    let find=|text:&str|tree.nodes.iter().find(|n|n.text.trim_end_matches(':')==text.trim_end_matches(':')).unwrap();
    let before=crate::StructureBodyWork {candidates:0,memo_entries:0,next_resolution_id:0,next_preparation_attempt:10};
    let mut replay=crate::StructureBodyReplay {before,body:crate::PortableStructureBody::freeze(&StructureBody::Trigger(Vec::new()),&mut |_,_|false).unwrap(),
        post_context:ExpressionParseContext::default(),work:crate::StructureBodyWork {next_preparation_attempt:13,..before},
        preparations:vec![preparation(&source,find("if x:"),10,1),preparation(&source,find("if y:"),11,10),preparation(&source,find("if z:"),12,1)]};
    assert!(valid_body_preparations(&source,&tree,root,1,&replay));
    replay.preparations[2].parent_attempt_id=Some(10);
    assert!(!valid_body_preparations(&source,&tree,root,1,&replay));
    replay.preparations[2].parent_attempt_id=Some(1);
    replay.preparations[2].candidate.matched.matched.span.mapped.virtual_range.start+=1;
    assert!(!valid_body_preparations(&source,&tree,root,1,&replay));
}
