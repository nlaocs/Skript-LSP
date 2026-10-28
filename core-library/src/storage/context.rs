//! Provider-owned scope evidence. Context text alone has no ownership.
use crate::nlaocs::skript_parser_addon::types::{ContextUpdate, HookOutput, HookPayload, InvocationContext,
    RegisteredExpressionPayload, StateEncoding, StateNamespaceVisibility, StateScope, StateValue};
use serde::{Serialize, Deserialize};
use std::collections::BTreeMap;

pub(crate) const NAMESPACE: &str = "storage-context";
pub(crate) const SCHEMA: &str = "nlaocs.core-library.storage-context.v2";
const KEY: &str = "core.storage.context-receipt";

#[derive(Serialize,Deserialize)]
struct Receipt {
    model:String,basis:String,premise:Option<String>, document: String, revision: u64, syntax_context: u64,
    snapshot: String, digest: String, expansion: Option<u64>,
    start: u64, end: u64, event_classes: Vec<String>, values: BTreeMap<String,String>,
}

fn relevant(key: &str) -> bool { key.starts_with("core.command.") || key=="core.storage.event-registration" }

/// Called only after a Core Structure handler has accepted this exact source.
/// Private state participates in host transaction rollback and Parse lifetime.
#[cfg(target_arch="wasm32")]
pub(crate) fn publish(context: &InvocationContext, output: &mut HookOutput) {
    use crate::nlaocs::skript_parser_addon::state_store;
    let Some(HookPayload::Structure(payload))=output.replacement.as_ref() else { return; };
    let Some(class)=payload.candidate.element_class.as_deref() else { return; };
    if !crate::catalog::standard_structure_source(&payload.candidate,class) { return; }
    let Some(root)=payload.body_tree.nodes.iter().find(|n|n.id==payload.candidate.raw_node_id) else { return; };
    let Some(body)=root.body_span.as_ref() else { return; };
    if body.virtual_range.start>=body.virtual_range.end { return; }
    let mut event_classes=None;
    let mut values=BTreeMap::new();
    for update in &output.effects.context_updates {
        if update.syntax_context!=context.syntax_context { continue; }
        if update.key=="parser.event-classes" {
            event_classes=update.value.as_deref().and_then(|v|std::str::from_utf8(v).ok())
                .map(|v|v.split(';').filter(|s|!s.is_empty()).map(str::to_owned).collect::<Vec<_>>());
        } else if relevant(&update.key) {
            let Some(value)=update.value.as_deref().and_then(|v|std::str::from_utf8(v).ok()) else { return; };
            values.insert(update.key.clone(),value.to_owned());
        }
    }
    let Some(event_classes)=event_classes else { return; };
    // Warm only immutable, source-bound proof inputs here. The body hook then
    // need not deserialize the registry in addition to its effect payload.
    // This performs no state write and never grants a context receipt itself.
    if !super::backend::prepare(){return;}
    let Some(basis)=super::backend::context_basis(payload,&event_classes)else{return;};
    let event_registration=basis.event_registration;
    if let Some(registration)=event_registration{values.insert("core.storage.event-registration".into(),registration.into());}
    let Some(source)=crate::nlaocs::skript_parser_addon::catalog_data::source().ok().flatten() else { return; };
    let receipt=Receipt {model:basis.model.into(),basis:basis.basis.into(),premise:basis.premise.map(str::to_owned),expansion:context.expansion,snapshot:source.snapshot_id,digest:source.source_digest,document:context.document_id.clone(),revision:context.document_revision,
        syntax_context:context.syntax_context,start:body.virtual_range.start,end:body.virtual_range.end,event_classes,values};
    let Ok(bytes)=serde_json::to_vec(&receipt) else { return; };
    if bytes.len()>64*1024 { return; }
    let key=serde_json::json!([context.document_id,context.document_revision,context.syntax_context,payload.candidate.raw_node_id]).to_string();
    if key.len()>4096 { return; }
    if state_store::put(StateScope::Parse,StateNamespaceVisibility::Private,NAMESPACE,&key,
        &StateValue {schema_id:SCHEMA.into(),encoding:StateEncoding::Json,bytes}).is_err() { return; }
    if let Some(registration)=event_registration {output.effects.context_updates.push(ContextUpdate {syntax_context:context.syntax_context,key:"core.storage.event-registration".into(),value:Some(registration.as_bytes().to_vec())});}
    output.effects.context_updates.push(ContextUpdate {syntax_context:context.syntax_context,key:KEY.into(),value:Some(key.into_bytes())});
}

#[cfg(not(target_arch="wasm32"))]
pub(crate) fn publish(_context: &InvocationContext, _output: &mut HookOutput) {}

fn matches(receipt: &Receipt, context: &InvocationContext, payload: &RegisteredExpressionPayload) -> bool {
    let values=payload.context.values.iter().filter(|v|relevant(&v.key)).collect::<Vec<_>>();
    let map=values.iter().map(|v|(v.key.clone(),v.value.clone())).collect::<BTreeMap<_,_>>();
    values.len()==map.len() && check(receipt,context,payload.context.syntax_context,
        payload.span.virtual_range.start,payload.span.virtual_range.end,&payload.context.event_classes,&map)
}

fn check(receipt: &Receipt, context: &InvocationContext, syntax_context: u64, start: u64, end: u64, events: &[String], values: &BTreeMap<String,String>) -> bool {
    let captured = super::evidence::ScopeRef { document:&receipt.document, revision:receipt.revision,
        syntax_context:receipt.syntax_context, expansion:receipt.expansion, start:receipt.start, end:receipt.end };
    let current = super::evidence::ScopeRef { document:&context.document_id, revision:context.document_revision,
        syntax_context, expansion:context.expansion, start, end };
    captured.covers(current) && context.syntax_context==syntax_context
        && receipt.event_classes==events
        && receipt.values==*values
}

#[cfg(target_arch="wasm32")]
fn valid(context: Option<&InvocationContext>, payload: &RegisteredExpressionPayload) -> bool {
    use crate::nlaocs::skript_parser_addon::state_store;
    let Some(context)=context else { return false; };
    let keys=payload.context.values.iter().filter(|v|v.key==KEY).collect::<Vec<_>>();
    if keys.len()!=1 || keys[0].value.len()>4096 { return false; }
    let Ok(Some(value))=state_store::get(StateScope::Parse,StateNamespaceVisibility::Private,NAMESPACE,&keys[0].value) else { return false; };
    if value.schema_id!=SCHEMA || !matches!(value.encoding,StateEncoding::Json) || value.bytes.len()>64*1024 { return false; }
    let Some(source)=crate::nlaocs::skript_parser_addon::catalog_data::source().ok().flatten() else { return false; };
    serde_json::from_slice::<Receipt>(&value.bytes).ok().is_some_and(|r|r.snapshot==source.snapshot_id && r.digest==source.source_digest && super::backend::current_basis().is_some_and(|b|b.matches(&r.model,&r.basis,r.premise.as_deref(),r.values.get("core.storage.event-registration").map(String::as_str))) && matches(&r,context,payload))
}

#[cfg(not(target_arch="wasm32"))]
fn valid(_context: Option<&InvocationContext>, _payload: &RegisteredExpressionPayload) -> bool { false }

pub(crate) fn retain_certificates(context: Option<&InvocationContext>, payload: &RegisteredExpressionPayload, resolution: &mut crate::expressions::SemanticResolution) {
    let crate::expressions::SemanticResolution::Resolved { metadata,.. }=resolution else { return; };
    if metadata.iter().any(|m|m.key=="skript.semantic.storage-provenance") && !valid(context,payload) {
        metadata.retain(|m|m.key!=super::METADATA_KEY && m.key!="skript.semantic.storage-provenance");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_scope_receipts_reject_context_forgery_revision_and_owner_range_reuse() {
        let context=InvocationContext {invocation_id:1,subscription_id:"fixture".into(),document_id:"file:///public.sk".into(),document_revision:2,expansion:None,syntax_context:0};
        let events=vec!["fixture.Event".into()];
        let values=BTreeMap::from([("core.command.argument.0.class".into(),"fixture.Native".into())]);
        let receipt=Receipt {model:"fixture.model".into(),basis:"fixture.basis".into(),premise:None,document:context.document_id.clone(),revision:2,syntax_context:0,snapshot:"fixture.snapshot".into(),digest:"fixture.source".into(),expansion:None,start:20,end:100,event_classes:events.clone(),values:values.clone()};
        assert!(check(&receipt,&context,0,30,40,&events,&values));
        assert!(!check(&receipt,&context,0,0,10,&events,&values));
        assert!(!check(&receipt,&context,0,40,101,&events,&values));
        assert!(!check(&receipt,&context,1,30,40,&events,&values));
        assert!(!check(&receipt,&context,0,30,40,&[],&values));
        let mut changed=values.clone(); changed.insert("core.command.argument.0.class".into(),"addon.Foreign".into());
        assert!(!check(&receipt,&context,0,30,40,&events,&changed));
        let mut newer=context.clone(); newer.document_revision=3;
        assert!(!check(&receipt,&newer,0,30,40,&events,&values));
        newer=context.clone(); newer.expansion=Some(7);
        assert!(!check(&receipt,&newer,0,30,40,&events,&values));
        newer=context.clone(); newer.document_id="file:///other.sk".into();
        assert!(!check(&receipt,&newer,0,30,40,&events,&values));
    }
}
