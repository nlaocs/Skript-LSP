//! Entry facts belong to the accepted header, not to every future body writer.
//! The draft is a request; only the current private receipt registry grants it.
use super::*;
use serde::Deserialize;

pub(in crate::host) const CAPABILITY:&str=crate::CAPABILITY_SEMANTIC_ENTRY_FACTS;
pub(in crate::host) const DRAFT:&str="parser.semantic.entry-facts.v1";
const MAX_ENTRIES:usize=256;

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Draft {version:u32,metadata_keys:Vec<String>}
#[derive(Debug)]
struct Entry {
    issuer:String,span:super::super::MappedSpan,header:String,
    definition:String,registration:String,pattern:u64,metadata:Vec<WitMetadataEntry>,
}
#[derive(Debug,Default)]
pub(in crate::host) struct Registry {entries:BTreeMap<u64,Entry>}

fn selected(metadata:&[WitMetadataEntry],issuer:&str)->Option<Vec<WitMetadataEntry>> {
    let mut drafts=metadata.iter().filter(|m|m.key==DRAFT);
    let draft=drafts.next()?;
    if drafts.next().is_some() || draft.owner_component_id.as_deref()!=Some(issuer) || draft.value.len()>16*1024{return None;}
    let draft:Draft=serde_json::from_str(&draft.value).ok()?;
    if draft.version!=1 || draft.metadata_keys.is_empty() || draft.metadata_keys.len()>16
        || draft.metadata_keys.iter().collect::<BTreeSet<_>>().len()!=draft.metadata_keys.len(){return None;}
    let mut bytes=0usize;let mut entries=Vec::new();
    for key in draft.metadata_keys {
        if key.is_empty() || key.len()>256 || key.starts_with("parser."){return None;}
        let mut values=metadata.iter().filter(|m|m.key==key);let value=values.next()?;
        bytes=bytes.checked_add(value.value.len())?;
        if values.next().is_some() || value.owner_component_id.as_deref()!=Some(issuer) || bytes>16*1024{return None;}
        entries.push(value.clone());
    }
    Some(entries)
}
impl Registry {
    pub(in crate::host) fn record(&mut self,token:u64,payload:&HookPayload,issuer:&str) {
        let HookPayload::Structure(payload)=payload else{return;};
        if !matches!(payload.timing,crate::bindings::nlaocs::skript_parser_addon::types::StructureTiming::EnterBody)
            || !matches!(payload.candidate.body_mode,crate::bindings::nlaocs::skript_parser_addon::types::StructureBodyMode::Trigger)
            || payload.candidate.declarations.is_empty() || payload.candidate.declarations.len()>16
            || self.entries.len()>=MAX_ENTRIES{return;}
        let candidate=&payload.candidate;
        let Some(metadata)=selected(&candidate.metadata,issuer) else{return;};
        let span=&candidate.span;
        // The producer owns the meaning of its selected entry metadata. The
        // host additionally binds document declarations to this current header;
        // successful native registration and final adoption remain mandatory.
        if candidate.declarations.iter().any(|declaration| {
            let crate::bindings::nlaocs::skript_parser_addon::types::ParserDeclaration::DocumentFunction(d)=declaration;
            d.source!=payload.input || d.span.start<span.virtual_range.start
                || d.span.end>span.virtual_range.end || d.span.start>=d.span.end
                || usize::try_from(d.span.start).ok().zip(usize::try_from(d.span.end).ok())
                    .is_none_or(|(a,b)|payload.input.get(a..b).is_none())
        }) {return;}
        if span.origins.len()!=1 || span.origins[0].kind!=super::super::WitOriginKind::Exact || span.origins[0].expansion.is_some(){return;}
        let Some((start,end))=usize::try_from(span.virtual_range.start).ok().zip(usize::try_from(span.virtual_range.end).ok()) else{return;};
        let Some(header)=payload.input.get(start..end).filter(|s|!s.is_empty()&&s.len()<=16*1024&&!s.contains(['\n','\r'])) else{return;};
        // Native immutable raw owner + exact header. An identical spelling at
        // another source position cannot borrow this entry receipt.
        let mut owners=payload.body_tree.nodes.iter().filter(|raw|raw.code_span.as_ref().is_some_and(|code|
            code.virtual_range.start==span.virtual_range.start&&span.virtual_range.end<=code.virtual_range.end));
        let Some(owner)=owners.next() else{return;};
        if owner.id!=candidate.raw_node_id || owners.next().is_some(){return;}
        self.entries.insert(token,Entry {issuer:issuer.into(),span:span.clone(),header:header.into(),
            definition:candidate.definition_id.clone(),registration:candidate.registration_id.clone(),pattern:candidate.pattern_index,metadata});
    }
    pub(super) fn has_receipt(&self,token:u64)->bool {self.entries.contains_key(&token)}
    pub(super) fn certify(&self,tree:&AstTree,receipts:&ReceiptRegistry,retained:&BTreeSet<u64>,global:&BTreeSet<String>,own:&BTreeMap<u64,BTreeSet<String>>)->BTreeMap<u64,value_facts::FactClaims> {
        let mut claims=BTreeMap::<u64,value_facts::FactClaims>::new();
        for node in tree.nodes.iter().filter(|n|n.kind==super::super::SyntaxKind::Structure) {
            let mut roots=BTreeSet::new();if !metadata_receipts(&node.metadata,&mut roots){continue;}
            let mut seen=BTreeSet::new();let mut pending=roots.into_iter().collect::<Vec<_>>();let mut valid=true;
            // ExitBody may carry a new receipt. Follow only its actual header
            // parents, never admitted body/default subjects or copied tokens.
            while let Some(token)=pending.pop() {
                if !seen.insert(token){continue;}
                if seen.len()>4096 {valid=false;break;}
                let Some(receipt)=receipts.receipts.get(&token) else{valid=false;break;};
                if !receipt.valid || receipt.parents.iter().any(|parent|*parent>=token) {valid=false;break;}
                pending.extend(receipt.parents.iter().copied());
            }
            if !valid {continue;}
            for token in seen {
                let Some(entry)=self.entries.get(&token) else{continue;};let issuer=&entry.issuer;
                if !retained.contains(&token) || receipts.receipts.get(&token).is_none_or(|r|!r.valid||r.issuer!=*issuer)
                    || global.iter().any(|writer|writer!=issuer)
                    || own.get(&node.id).is_none_or(|writers|writers.iter().any(|writer|writer!=issuer))
                    || !value_facts::ancestor_authors(tree,node.id,own,issuer)
                    || !entry.matches(node) || tree.nodes.iter().filter(|n|entry.matches(n)).count()!=1
                    || entry.metadata.iter().any(|m|!node.metadata.iter().any(|current|current.key==m.key&&current.value==m.value&&current.owner_component_id==m.owner_component_id)){continue;}
                claims.entry(node.id).or_default().metadata.extend(entry.metadata.iter().map(|m|(issuer.clone(),m.key.clone())));
            }
        }
        claims
    }
}
impl Entry {
    fn matches(&self,node:&AstNode)->bool {
        node.kind==super::super::SyntaxKind::Structure && node.text==self.header
            && debug_digest("entry-span-v1",&node.span)==debug_digest("entry-span-v1",&self.span)
            && node.summary.as_ref().is_some_and(|s|s.kind=="structure"&&s.definition_id.as_ref()==Some(&self.definition)
                &&s.registration_id.as_ref()==Some(&self.registration)&&s.pattern_index==Some(self.pattern))
    }
}

#[cfg(test)]
mod tests;
