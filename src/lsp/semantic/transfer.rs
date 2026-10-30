//! Current-flow inputs to owner-authenticated transfer operations. No editor
//! proposal, historical fact or previous revision supplies a type here.
use super::*;
pub(super) use parser_wasm::host::semantic_transfer::{BindingEffects,Output,SourceUpdate,TypeBound};

pub(super) trait Resolver {
    fn evaluate(&mut self,node:u64,input:TypeBound)->Result<Option<Output>,String>;
    fn certificate(&self)->Option<parser_wasm::host::semantic_transfer::Certificate> {None}
}
#[cfg(test)]
impl<F> Resolver for F where F:FnMut(u64,TypeBound)->Result<Option<Output>,String> {
    fn evaluate(&mut self,node:u64,input:TypeBound)->Result<Option<Output>,String> {self(node,input)}
}
impl Resolver for parser_wasm::host::semantic_transfer::Session<'_> {
    fn evaluate(&mut self,node:u64,input:TypeBound)->Result<Option<Output>,String> {
        self.evaluate(node,input).map_err(|e|e.to_string())
    }
    fn certificate(&self)->Option<parser_wasm::host::semantic_transfer::Certificate> {self.certificate()}
}
pub(super) fn bound(ty:&InferredType)->Option<TypeBound> {
    let InferredType::Values {classes,multiple,maybe_unset,support}=ty else{return None;};
    if !support.is_proof()||*multiple||classes.len()>64||classes.is_empty()&&!*maybe_unset {return None;}
    Some(TypeBound {types:classes.iter().cloned().collect(),complete:true,multiple:false,maybe_unset:*maybe_unset})
}
pub(super) fn inferred(bound:&TypeBound)->Option<InferredType> {
    (bound.complete&&!bound.multiple&&bound.types.len()<=64&&(!bound.types.is_empty()||bound.maybe_unset)
        && bound.types.iter().all(|t|!t.is_empty()&&t.len()<=1024&&t!="java.lang.Object")
        && bound.types.iter().collect::<BTreeSet<_>>().len()==bound.types.len())
        .then(||InferredType::Values {classes:bound.types.iter().cloned().collect(),multiple:false,maybe_unset:bound.maybe_unset,
            support:TypeSupport::from(Evidence::DeclaredTransfer)})
}

#[derive(Clone,Copy,PartialEq,Eq)]
pub(super) enum Use {Flow,Completion}
fn usage(node:&AstNode)->Option<Use> {
    let data:serde_json::Value=serde_json::from_str(unique_metadata(node,parser_wasm::host::semantic_transfer::METADATA)?).ok()?;
    match (data["version"].as_u64(),data["outputUse"].as_str()) {
        (Some(1),None)|(Some(2),Some("flow"))=>Some(Use::Flow),
        (Some(2),Some("completion-candidate"))=>Some(Use::Completion),
        _=>None,
    }
}
impl Engine<'_,'_> {
    pub(super) fn transfer(&mut self,node:&AstNode,ty:&InferredType,expected:Use)->Result<Option<Output>,String> {
        if usage(node)!=Some(expected){return Ok(None);}
        let Some(input)=bound(ty) else{return Ok(None);};
        if unique_metadata(node,parser_wasm::host::semantic_transfer::METADATA).is_none() {return Ok(None);}
        let Some(resolver)=self.transfer_resolver.as_deref_mut() else{return Ok(None);};
        let output=resolver.evaluate(node.id,input)?;
        if output.is_some() {self.document.transfer_certificate=resolver.certificate();}
        if output.as_ref().is_some_and(|o|inferred(&o.value).is_none()
            || matches!(&o.source,SourceUpdate::Known(b)if inferred(b).is_none())) {return Err("invalid complete transfer bound".into());}
        Ok(output)
    }
    pub(super) fn apply_read_transfer(&mut self,identity:&Identity,output:&Output,previous:Option<Fact>,range:TextRange,facts:&mut Facts) {
        if matches!(output.effects,BindingEffects::Invalidate) {
            invalidate(facts,Some(range),Invalidation::Read);
        }
        match &output.source {
            SourceUpdate::Unchanged=>{},
            SourceUpdate::Unknown=>invalidate_where(facts,Some(range),Invalidation::Read,|key,fact|
                key==&identity.key||fact.list||may_alias(key,fact.alias.as_ref(),identity)),
            SourceUpdate::Known(bound)=>{
                invalidate_where(facts,Some(range),Invalidation::Alias,|key,fact|
                    key!=&identity.key&&(fact.list||may_alias(key,fact.alias.as_ref(),identity)));
                if let (Some(mut fact),Some(ty))=(previous,inferred(bound)) {
                    // A complete poststate type does not certify that the old
                    // authored write still reaches subsequent reads.
                    fact.ty=ty;fact.history=None;fact.definitions.clear();facts.insert(identity.key.clone(),fact);
                }
            },
        }
    }
}
