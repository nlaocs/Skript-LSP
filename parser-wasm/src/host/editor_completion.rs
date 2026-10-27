//! Nonsemantic, live-prefix editing queries over the existing custom-parser ABI.
use super::*;
use serde::{Deserialize, Serialize};

pub const ROUTE_PREFIX: &str = "editor.completion/";
pub const CAPABILITY: &str = "parser.editor-completion";
const RESULT: &str = "skript.editor.query-result.v1";
const MAX_QUERIES: usize = 128;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Proposal {
    pub id: String,
    pub label: String,
    pub filter_text: String,
    pub insert_text: String,
    pub snippet: Option<String>,
    pub start: usize,
    pub end: usize,
    pub detail: String,
    pub documentation: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Output {
    pub version: u32,
    pub complete: bool,
    pub exclusive: bool,
    pub proposals: Vec<Proposal>,
}
#[derive(Debug, Clone)]
pub struct Batch {
    pub owner: String,
    pub attempt: u64,
    pub input: String,
    pub span: MappedSpan,
    pub output: Output,
}
pub(super) struct Session {
    pub source: Arc<str>,
    pub batches: Vec<Batch>,
    parents: BTreeMap<u64,Option<u64>>,
    queries: usize,
    bytes: usize,
}
impl Session {
    pub(super) fn new(source:Arc<str>)->Self {
        Self {source,batches:Vec::new(),parents:BTreeMap::new(),queries:0,bytes:0}
    }
    pub(super) fn preparation(&mut self,event:&skript_parser::StructurePreparationEvent<'_>) {
        use skript_parser::StructurePreparationEvent::*;
        match event {
            Headers {headers,..}=>for h in headers {if !h.ambiguous {self.parents.insert(h.attempt_id,None);}},
            StructureHeader {header,..}=>{if !header.ambiguous {self.parents.insert(header.attempt_id,None);}},
            SectionHeader {attempt_id,parent_attempt_id,..}=>{self.parents.insert(*attempt_id,*parent_attempt_id);},
            Invalidated {attempt_id,..}=>{
                let mut removed=BTreeSet::from([*attempt_id]);
                loop {let before=removed.len();for (id,parent) in &self.parents {if parent.is_some_and(|p|removed.contains(&p)){removed.insert(*id);}}if before==removed.len(){break;}}
                self.batches.retain(|b|!removed.contains(&b.attempt));
                self.parents.retain(|id,_|!removed.contains(id));
            }
        }
    }
}

fn valid_output(output:&Output,input:&str)->bool {
    output.version==1 && output.exclusive && output.proposals.len()<=256 && output.proposals.iter().all(|p|
        !p.id.is_empty() && p.id.len()<=256 && !p.id.chars().any(char::is_control)
        && !p.label.is_empty() && p.label.len()<=512 && !p.label.chars().any(char::is_control)
        && !p.filter_text.is_empty() && p.filter_text.len()<=256 && !p.filter_text.chars().any(char::is_control)
        && !p.insert_text.is_empty() && p.insert_text.len()<=1024 && !p.insert_text.chars().any(char::is_control)
        && p.snippet.as_ref().is_none_or(|s|!s.is_empty()&&s.len()<=2048&&!s.chars().any(char::is_control))
        && p.detail.len()<=1024 && !p.detail.contains('\0') && p.documentation.len()<=4096 && !p.documentation.contains('\0')
        && p.start<=p.end && input.get(p.start..p.end).is_some())
}

impl ParserHost {
    pub(super) fn editor_routes(&self)->Vec<(String,String,char)> {
        self.components.iter().flat_map(|c|{
            let prefix=format!("{ROUTE_PREFIX}{}/",c.manifest.component_id);
            c.manifest.subscriptions.iter().filter_map(move |s|{
                let HookTarget::Parser(route)=&s.target else{return None};
                let trigger=route.strip_prefix(&prefix)?;let mut chars=trigger.chars();let trigger=chars.next()?;
                (chars.next().is_none()&&!trigger.is_alphanumeric()&&!trigger.is_whitespace()&&!trigger.is_control())
                    .then(||(c.manifest.component_id.clone(),route.clone(),trigger))
            })
        }).collect()
    }
    pub(super) fn editor_line(&mut self,transaction:&ParseTransaction,invocation:&InvocationContext,
        input:&str,span:&MappedSpan,context:&ExpressionParseContext,attempt:u64)->Result<(),HostError> {
        if self.editor_context.is_some() || input.len()>4096 {return Ok(());}
        let Some(session)=self.editor_session.as_ref() else{return Ok(())};
        if !session.parents.contains_key(&attempt) || session.queries>=MAX_QUERIES || session.bytes>=MAX_BYTES {return Ok(());}
        let [origin]=span.origins.as_slice() else{return Ok(())};
        if origin.kind!=WitOriginKind::Exact || origin.original_range.end<origin.original_range.start
            || session.source.get(origin.original_range.start as usize..origin.original_range.end as usize)!=Some(input) {return Ok(());}
        let routes=self.editor_routes().into_iter().filter(|(_,_,trigger)|input.contains(*trigger)).collect::<Vec<_>>();
        for (owner,route,_) in routes {
            self.check_native_cancellation()?;
            if self.editor_session.as_ref().is_none_or(|s|s.queries>=MAX_QUERIES||s.bytes>=MAX_BYTES){break;}
            self.editor_session.as_mut().unwrap().queries+=1;
            let request=inherit_parse_request_context(ParseRequest {request_id:0,parser_id:route.clone(),input:input.into(),
                expected_types:Vec::new(),span:span.clone(),options:vec![WitMetadataEntry {key:"editor.query-version".into(),value:"1".into(),owner_component_id:None}]},Some(&parse_context_to_wit(context)));
            let before=transaction.savepoint()?;
            let observations=transaction.isolate_editor_observations()?;
            let receipts=self.semantic_receipts.replace(semantic_proof::ReceiptRegistry::with_sequence(self.semantic_receipt_sequence.clone()));
            let dependencies=self.native_dependency_unavailable.replace(Vec::new());
            let incremental=mem::take(&mut self.incremental);
            let observing=mem::replace(&mut self.body_observing,false);
            let impure=self.body_impure;
            self.editor_context=Some(context.clone());
            for component in &mut self.components {component.store.data_mut().editor_readonly=true;component.store.data_mut().editor_mutation_attempted=false;}
            let parsed=self.execute_addon_parse_request(transaction,invocation,&request);
            let mutation=self.components.iter().any(|c|c.store.data().editor_mutation_attempted);
            for component in &mut self.components {component.store.data_mut().editor_readonly=false;component.store.data_mut().editor_mutation_attempted=false;}
            self.editor_context=None;
            self.semantic_receipts=receipts;
            let unavailable=mem::replace(&mut self.native_dependency_unavailable,dependencies).is_some_and(|v|!v.is_empty());
            self.incremental=incremental;
            self.body_observing=observing;
            // A cached body does not contain a replayable editing ledger. Only
            // trigger-bearing bodies are ineligible; unrelated bodies can hit.
            self.body_impure=impure||observing;
            let rollback=transaction.rollback_to(&before);
            let restore=observations.restore();
            rollback?;restore?;
            let parsed=parsed?; // Existing extension faults remain faults.
            self.check_native_cancellation()?;
            if mutation||unavailable||parsed.wire.status!=WitParseResultStatus::Success {continue;}
            if !parsed.calls.iter().any(|c|c.component_id==owner) {continue;}
            let [root]=parsed.wire.roots.as_slice() else{continue};
            let [node]=parsed.wire.nodes.as_slice() else{continue};
            if node.node_id!=*root||node.kind!="editor-query-result"||node.summary.is_some()||node.function.is_some()
                ||!node.children.is_empty()||!node.diagnostics.is_empty()||!parsed.wire.diagnostics.is_empty(){continue;}
            let entries=node.metadata.iter().filter(|m|m.key==RESULT).collect::<Vec<_>>();
            let [entry]=entries.as_slice() else{continue};
            if entry.value.len()>128*1024 {continue;}
            let Ok(output)=serde_json::from_str::<Output>(&entry.value) else{continue};
            if !valid_output(&output,input){continue;}
            let session=self.editor_session.as_mut().unwrap();
            if session.bytes.saturating_add(entry.value.len())>MAX_BYTES {continue;}
            session.bytes+=entry.value.len();
            session.batches.push(Batch {owner,attempt,input:input.into(),span:span.clone(),output});
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn editing_ranges_and_payload_limits_fail_closed(){
        let mut output=Output {version:1,complete:true,exclusive:true,proposals:vec![Proposal{id:"m".into(),label:"name()".into(),filter_text:"name".into(),insert_text:"name()".into(),snippet:None,start:2,end:2,detail:String::new(),documentation:String::new()}]};
        assert!(valid_output(&output,"\u{e9}."));output.proposals[0].start=1;assert!(!valid_output(&output,"\u{e9}."));
        output.proposals[0].start=2;output.proposals[0].insert_text="x\ny".into();assert!(!valid_output(&output,"\u{e9}."));
    }
    #[test]fn rejecting_ancestor_revokes_only_its_descendant_editing_queries(){
        let mut s=Session::new(Arc::from("x"));
        s.parents=BTreeMap::from([(1,None),(2,Some(1)),(3,Some(2)),(4,None)]);
        for attempt in [1,2,3,4]{s.batches.push(Batch {owner:"owned".into(),attempt,input:"x".into(),span:MappedSpan {virtual_range:WitTextRange {start:0,end:1},origins:Vec::new()},output:Output {version:1,complete:true,exclusive:true,proposals:Vec::new()}});}
        s.preparation(&skript_parser::StructurePreparationEvent::Invalidated {attempt_id:1,raw_node_id:skript_parser::RawNodeId::new(0),registration_id:"test",pattern_index:0});
        assert_eq!(s.batches.iter().map(|b|b.attempt).collect::<Vec<_>>(),vec![4]);assert_eq!(s.parents.len(),1);
    }

}
