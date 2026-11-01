//! Current-Structure, direct-entry body plans. These describe parser scope, not runtime truth.
use super::*;
use serde::Deserialize;

/// Host-issued immutable layout lease carried by the current Structure invocation.
pub const STRUCTURE_ENTRY_LAYOUT_METADATA: &str = "host.structure-entry-layout.v1";
/// Component-owned direct-entry parser and context plan.
pub const STRUCTURE_ENTRY_PLANS_METADATA: &str = "structure.entry-plans.v1";
const MAX_BYTES: usize = 16 * 1024;
const MAX_ENTRIES: usize = 32;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructureEntryContext {
    pub event_classes: Option<Vec<ClassName>>,
    pub values: BTreeMap<String, Option<String>>,
}
impl StructureEntryContext {
    pub fn apply(&self, original: &ExpressionParseContext) -> ExpressionParseContext {
        let mut context = original.clone();
        if let Some(classes) = &self.event_classes { context.event_classes = classes.clone(); }
        for (key, value) in &self.values {
            if let Some(value) = value { context.values.insert(key.clone(), value.clone()); }
            else { context.values.remove(key); }
        }
        context
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureEntryPlan {
    pub declaration_index: usize,
    pub mode: crate::SectionBodyMode,
    pub adopt_unknown: bool,
    pub context: StructureEntryContext,
}

/// Decoded once, then validated against the current native tree and validator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructureEntryPlans {
    pub owner: Option<String>,
    pub entries: BTreeMap<RawNodeId, StructureEntryPlan>,
}

#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct WirePlans { version: u32, token: String, structure_node: u64, entries: Vec<WireEntry> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct WireEntry { node: u64, declaration_index: usize, mode: String, adopt_unknown: bool, context: WireContext }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct WireContext { event_classes: Option<Vec<String>>, values: Vec<WireValue> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireValue { key: String, value: Option<String> }

/// Issues a bounded validator descriptor. A lease is fresh for each host EnterBody.
/// Indexes refer to root EntryData declarations, never flattened parsed values.
pub fn structure_entry_layout(candidate: &StructureCandidate, token: &str) -> Result<String, String> {
    let validator = candidate.entry_validator.as_ref().ok_or("entry layout requires a validator")?;
    if validator.entry_data.len() > 128 { return Err("entry layout declaration limit".into()); }
    let entries = validator.entry_data.iter().enumerate().map(|(index, data)| {
        let kind = match data.kind {
            EntryKind::Section => "section", EntryKind::Trigger => "trigger", EntryKind::Unknown => "unknown",
            EntryKind::Container => "container", _ => "value",
        };
        serde_json::json!({"index":index,"key":data.key,"kind":kind,"entryDataClass":data.entry_data_class.as_str(),"multiple":data.multiple})
    }).collect::<Vec<_>>();
    let value = serde_json::json!({"version":1,"token":token,"structureNode":candidate.raw_node_id.get(),"entries":entries}).to_string();
    if value.len() > MAX_BYTES { return Err("entry layout byte limit".into()); }
    Ok(value)
}

/// Checks ownership, current host lease, shape and budgets without parsing a body.
/// The host must stamp component metadata and protect the ownerless layout first.
pub fn decode_structure_entry_plans(metadata: &BTreeMap<String,String>, root: RawNodeId) -> Result<StructureEntryPlans,String> {
    let mut records = metadata.iter().filter_map(|(key,value)| {
        let (owner,key) = key.split_once('/')?;
        (key == STRUCTURE_ENTRY_PLANS_METADATA).then_some((owner,value))
    });
    if metadata.contains_key(STRUCTURE_ENTRY_PLANS_METADATA) { return Err("entry plans require an owning component".into()); }
    let Some((owner, value)) = records.next() else { return Ok(StructureEntryPlans::default()); };
    if owner.is_empty() || records.next().is_some() { return Err("entry plans require exactly one component owner".into()); }
    if value.len() > MAX_BYTES { return Err("entry plan byte limit".into()); }
    let wire: WirePlans = serde_json::from_str(value).map_err(|e|format!("invalid entry plan: {e}"))?;
    let layout_text = metadata.get(STRUCTURE_ENTRY_LAYOUT_METADATA).ok_or("entry plans lack a current native layout")?;
    if layout_text.len() > MAX_BYTES { return Err("entry layout byte limit".into()); }
    let layout: serde_json::Value = serde_json::from_str(layout_text).map_err(|e|e.to_string())?;
    if wire.version != 1 || wire.structure_node != root.get() || wire.token.is_empty()
        || layout["version"] != 1 || layout["structureNode"] != root.get()
        || layout["token"].as_str() != Some(wire.token.as_str()) {
        return Err("entry plan has a stale or foreign native layout lease".into());
    }
    if wire.entries.len() > MAX_ENTRIES { return Err("entry plan count limit".into()); }
    let mut output = StructureEntryPlans {owner:Some(owner.to_owned()),entries:BTreeMap::new()};
    for entry in wire.entries {
        let mode = match entry.mode.as_str() {"trigger"=>crate::SectionBodyMode::Trigger,"conditions"=>crate::SectionBodyMode::Conditions,_=>return Err("invalid entry plan body mode".into())};
        let classes = entry.context.event_classes.map(|values| {
            if values.len() > 8 || values.iter().any(|v|v.is_empty() || v.len()>512) { return Err("entry event class limit".to_owned()); }
            Ok(values.into_iter().map(ClassName).collect())
        }).transpose()?;
        if entry.context.values.len() > 16 { return Err("entry context count limit".into()); }
        let mut values = BTreeMap::new();
        for value in entry.context.values {
            if value.key.trim().is_empty() || value.key.len()>128 || value.key=="parser.event-classes"
                || value.value.as_ref().is_some_and(|v|v.len()>1024)
                || values.insert(value.key,value.value).is_some() { return Err("invalid or duplicate entry context value".into()); }
        }
        if output.entries.insert(RawNodeId::new(entry.node), StructureEntryPlan {
            declaration_index:entry.declaration_index,mode,adopt_unknown:entry.adopt_unknown,
            context:StructureEntryContext {event_classes:classes,values},
        }).is_some() { return Err("duplicate entry plan node".into()); }
    }
    Ok(output)
}

/// Validates all direct claims before evaluating any body, including claims for
/// later children. A duplicate native single entry is never an unhandled entry.
pub fn validated_structure_entry_plans(candidate: &StructureCandidate, tree: &RawTree, mode: StructureBodyMode) -> Result<StructureEntryPlans,String> {
    let plans = decode_structure_entry_plans(&candidate.metadata,candidate.raw_node_id)?;
    if plans.owner.is_none() { return Ok(plans); }
    let root = tree.get(candidate.raw_node_id).ok_or("entry plan Structure is absent")?;
    let validator = candidate.entry_validator.as_ref().ok_or("entry plans require a native validator")?;
    if mode != StructureBodyMode::Entries || root.kind != RawNodeKind::Section { return Err("entry plans require an Entries Section".into()); }
    let mut seen = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for id in &root.children {
        let child = tree.get(*id).ok_or("entry plan child is absent")?;
        if matches!(child.kind,RawNodeKind::Blank|RawNodeKind::Comment) { continue; }
        let selected = structure_entry_index(validator,child,&seen);
        let index = if let Some(plan) = plans.entries.get(id) {
            visited.insert(*id);
            let data = validator.entry_data.get(plan.declaration_index).ok_or("entry plan declaration index is absent")?;
            if child.kind != RawNodeKind::Section || child.parent != Some(root.id)
                || !data.multiple && seen.contains(&plan.declaration_index)
                || selected.is_some_and(|index|index!=plan.declaration_index)
                || selected.is_none() && validator.entry_data.iter().any(|data|structure_entry_matches(data,child)) {
                return Err("entry plan steals a selected, duplicate, or foreign entry".into());
            }
            match data.kind {
                EntryKind::Unknown if plan.adopt_unknown => {},
                EntryKind::Section if !plan.adopt_unknown && selected==Some(plan.declaration_index) => {},
                EntryKind::Trigger if !plan.adopt_unknown && selected==Some(plan.declaration_index) && plan.mode==crate::SectionBodyMode::Trigger => {},
                _ => return Err("entry plan cannot adopt this EntryData kind".into()),
            }
            if candidate.entry_body_modes.contains_key(&vec![data.key.clone()]) { return Err("entry plan conflicts with an entry body mode".into()); }
            Some(plan.declaration_index)
        } else { selected };
        if let Some(index) = index { seen.insert(index); }
    }
    if visited.len()!=plans.entries.len() { return Err("entry plan references a missing, nested, or foreign Section".into()); }
    Ok(plans)
}

/// Current direct-child selector shared by parsing and provisional editor layout.
pub fn structure_entry_index_with_plans(validator:&EntryValidator,node:&RawNode,seen:&BTreeSet<usize>,plans:&StructureEntryPlans)->Option<usize> {
    plans.entries.get(&node.id).map(|plan|plan.declaration_index)
        .filter(|index|validator.entry_data.get(*index).is_some_and(|data|data.multiple || !seen.contains(index)))
        .or_else(||structure_entry_index(validator,node,seen))
}

#[cfg(test)]
mod tests;
