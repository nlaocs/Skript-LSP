//! Editing observations use the existing ordered flow pass, never occurrence history.
use super::*;
use parser_wasm::host::{EditorCompletionInput as Input, EditorCompletionTypeFact as TypeFact};

fn projection(input: &Input, source: &skript_parser::MappedSource, anchor: bool) -> Option<AstTree> {
    if !input.is_current() { return None; }
    let original=if anchor {input.anchor()?} else {input.subject()};
    let witnesses=if anchor {input.anchor_witnesses(source)?} else {input.subject_witnesses(source)};
    let mut tree=original.clone();
    for (node,original) in tree.nodes.iter_mut().zip(&original.nodes) {
        let native=witnesses.native_valid(original);
        let issuer=witnesses.unique_issuer(original);
        if !native && issuer.is_none() {return None;}
        let composed=original.metadata.iter().filter(|entry|witnesses.composed_metadata_valid(original,entry))
            .filter_map(|entry|entry.owner_component_id.as_ref().map(|owner|(owner.clone(),entry.key.clone()))).collect();
        let public=witnesses.composed_public_data_valid(original,IDENTITY);
        retain_semantic_contracts_with_composition(node,issuer,&composed);
        if issuer.is_none() && !public {
            if let Some(summary)=&mut node.summary {summary.public_data.retain(|record|record.schema_id!=IDENTITY);}
        }
        let range=exact_span(&node.span)?;
        let virtual_range=TextRange::new(node.span.virtual_range.start.try_into().ok()?,node.span.virtual_range.end.try_into().ok()?);
        if source.original().get(range.start..range.end)?!=node.text
            || source.virtual_source().get(virtual_range.start..virtual_range.end)?!=node.text {return None;}
    }
    Some(tree)
}

fn scalar_read<'a>(tree:&'a AstTree, scope:u64) -> Option<(&'a AstNode,Identity,VariableRead)> {
    let [root]=tree.roots.as_slice() else{return None;};
    let mut node=tree.nodes.iter().find(|node|node.id==*root)?;
    for _ in 0..16 {
        if node.summary.as_ref()?.kind!="grouped-expression" {break;}
        let [child]=node.children.as_slice() else{return None;};
        node=tree.nodes.iter().find(|node|node.id==*child)?;
    }
    if node.summary.as_ref()?.kind!="variable" || !node.children.is_empty() {return None;}
    let mut records=node.summary.as_ref()?.public_data.iter().filter(|record|record.schema_id==IDENTITY);
    let record=records.next()?;
    if records.next().is_some()||record.schema_version!=1 {return None;}
    let data:IdentityData=serde_json::from_str(&record.json).ok()?;
    let identity=identity_data(data,scope,node.syntax_context)?;
    if identity.list {return None;}
    let read:VariableRead=serde_json::from_str(unique_metadata(node,VARIABLE_READ)?).ok()?;
    (matches!(read.version,1|2|3)&&!read.requested_types.is_empty()).then_some((node,identity,read))
}

impl Engine<'_,'_> {
    pub(super) fn observe_editor_input(&mut self, input:&Input, scope:u64, facts:&Facts) {
        let Some(source)=self.mapped_source else{return;};
        let Some(tree)=projection(input,source,false) else{return;};
        let Some((subject,identity,read))=scalar_read(&tree,scope) else{return;};
        let Some(range)=exact_span(&subject.span) else{return;};
        if self.blocked.iter().any(|blocked|overlaps(*blocked,range))
            || !unique_projection(&self.ambiguous_projections,range) {return;}
        let Some(line_start)=usize::try_from(input.line_span().virtual_range.start).ok() else{return;};
        if let Some(anchor)=input.anchor() {
            let _=anchor;
            let Some(tree)=projection(input,source,true) else{return;};
            let [root]=tree.roots.as_slice() else{return;};
            let nodes=tree.nodes.iter().map(|node|(node.id,node)).collect::<HashMap<_,_>>();
            let Some(parent)=nodes.get(root) else{return;};
            if parent.span.virtual_range.start!=input.line_span().virtual_range.start {return;}
            let Some(plan)=operand_prefix(parent,&[],&nodes,self.evaluation_context.as_ref()) else{return;};
            let Some(first)=plan.steps.first() else{return;};
            let child_tree=AstTree {roots:vec![first.child],nodes:tree.nodes.clone()};
            let Some((first,first_identity,_))=scalar_read(&child_tree,scope) else{return;};
            if (first.span.virtual_range.start,first.span.virtual_range.end)!=(subject.span.virtual_range.start,subject.span.virtual_range.end)||first_identity.key!=identity.key
                || first_identity.alias!=identity.alias {return;}
        } else {
            let [root]=tree.roots.as_slice() else{return;};
            let Some(root)=tree.nodes.iter().find(|node|node.id==*root) else{return;};
            if root.span.virtual_range.start!=line_start as u64 {return;}
        }
        let Some(fact)=facts.get(&identity.key) else{return;};
        let current=identity.key.local||self.shared_isolated;
        let (classes,maybe_unset,inferred)=if current&&let Some((classes,false))=fact.ty.proven_types() {
            let InferredType::Values {maybe_unset,..}=&fact.ty else{return;};
            (classes,*maybe_unset,false)
        }else if !identity.key.local&&let Some((classes,maybe_unset))=shared::candidate_bound(fact) {
            (classes,maybe_unset,true)
        }else{return;};
        if classes.is_empty()||classes.len()>16 {return;}
        self.document.editor_type_facts.push(TypeFact {input:input.clone(),classes:classes.iter().cloned().collect(),complete:!inferred,
            multiple:false,maybe_unset,inferred,read_closed:!inferred&&read_preserves_type_facts(&read,&fact.ty,&self.document.catalog)});
    }
}
