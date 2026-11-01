//! Metadata transport for current native entry leases, never a durable authority.
use super::*;
use skript_parser::{STRUCTURE_ENTRY_LAYOUT_METADATA as LAYOUT, STRUCTURE_ENTRY_PLANS_METADATA as PLANS};

static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);

pub(super) fn input_metadata(request:&StructureHookRequest<'_>)->Result<Vec<WitMetadataEntry>,String> {
    let mut metadata=metadata_to_wit(&request.candidate.metadata);
    if request.timing==StructureHookTiming::EnterBody {
        reset_for_enter(&mut metadata);
    }
    if request.timing==StructureHookTiming::EnterBody && request.candidate.entry_validator.is_some() {
        let token=NEXT_LEASE.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|n|n.checked_add(1))
            .map_err(|_|"Structure entry lease space exhausted")?;
        metadata.retain(|entry|entry.key!=LAYOUT || entry.owner_component_id.is_some());
        // An unrelated large validator remains parseable under the old contract.
        // It simply cannot opt into this bounded direct-entry extension.
        let Ok(value)=skript_parser::structure_entry_layout(request.candidate,&token.to_string()) else {return Ok(metadata);};
        metadata.push(WitMetadataEntry {key:LAYOUT.into(),owner_component_id:None,value});
    }
    Ok(metadata)
}

pub(super) fn validate_owner_requirements(original:&HookPayload,next:&HookPayload,owner:&str,requirements:&[crate::bindings::nlaocs::skript_parser_addon::types::CapabilityRequirement])->Result<(),String> {
    let (HookPayload::Structure(original),HookPayload::Structure(next))=(original,next)else{return Ok(());};
    let added=next.candidate.metadata.iter().any(|entry|entry.key==PLANS && entry.owner_component_id.as_deref()==Some(owner)
        && !original.candidate.metadata.iter().any(|prior|prior.key==entry.key && prior.owner_component_id==entry.owner_component_id && prior.value==entry.value));
    if added {
        for capability in [crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,CAPABILITY_CONTEXT_UPDATES] {
            if !requirements.iter().any(|c|c.id==capability && c.required && c.minimum_version==1) {
                return Err(format!("entry plans require the mandatory {capability} v1 contract"));
            }
        }
    }
    Ok(())
}

fn decode(value:&WitStructurePayload)->Result<skript_parser::StructureEntryPlans,String> {
    let plans=skript_parser::decode_structure_entry_plans(&metadata_entries(value.candidate.metadata.clone())?,ParserRawNodeId::new(value.candidate.raw_node_id))?;
    if plans.owner.is_some() && (value.candidate.actual_node_type!=WitStructureNodeType::Section || value.candidate.body_mode!=WitStructureBodyMode::Entries) {
        return Err("entry plans require an Entries Section".into());
    }
    Ok(plans)
}

pub(super) fn validate_transition(previous:&WitStructurePayload,next:&WitStructurePayload)->Result<(),String> {
    let prior=decode(previous)?;
    let planned=decode(next)?;
    if previous.timing==WitStructureTiming::ExitBody && prior!=planned {
        return Err("Structure exit hook changed entry plans".into());
    }
    Ok(())
}

/// Only lexical Structure context reach is added. Existing Unbound effects
/// (additional parsing, shared State, staged syntax) can never be downgraded.
pub(super) fn changed(previous:&HookPayload,next:&HookPayload)->bool {
    let (HookPayload::Structure(previous),HookPayload::Structure(next))=(previous,next)else{return false;};
    let records=|value:&WitStructurePayload|value.candidate.metadata.iter().filter(|e|e.key==PLANS)
        .map(|e|(e.owner_component_id.clone(),e.value.clone())).collect::<Vec<_>>();
    previous.timing==WitStructureTiming::EnterBody && records(previous)!=records(next)
}

#[cfg(test)]
mod tests;

/// Drop a temporary native lease only when nobody supplied a plan. This is
/// called after checked EnterBody acceptance, never to hide malformed plans.
pub(super) fn discard_unused_layout(metadata:&mut Vec<WitMetadataEntry>) {
    if !metadata.iter().any(|entry|entry.key==PLANS) {
        metadata.retain(|entry|entry.key!=LAYOUT || entry.owner_component_id.is_some());
    }
}

/// An alternative/header retry must reacquire entry scope authority from all
/// current providers. Old plans cannot travel into a fresh EnterBody lease.
fn reset_for_enter(metadata:&mut Vec<WitMetadataEntry>) {
    metadata.retain(|entry|entry.key!=PLANS && (entry.key!=LAYOUT || entry.owner_component_id.is_some()));
}
