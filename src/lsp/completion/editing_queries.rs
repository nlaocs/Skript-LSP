//! Owner-stamped suggestions from an isolated live-context editing query.
use super::*;

impl Document {
    pub(super) fn editing_query_at(&self,line:&Line,offset:usize,snippets:bool)->Result<Option<(CompletionList,bool)>,String>{
        let mut items=Vec::new();let mut exclusive=false;let mut incomplete=false;let mut seen=HashSet::new();
        'batches: for batch in &self.editor_queries {
            let [origin]=batch.span.origins.as_slice() else{continue};
            let start=usize::try_from(origin.original_range.start).map_err(|_|"editing source overflow")?;
            let end=usize::try_from(origin.original_range.end).map_err(|_|"editing source overflow")?;
            if start<line.code_start||end>line.range.end||self.source.get(start..end)!=Some(batch.input.as_str()){continue;}
            for proposal in &batch.output.proposals {
                let range=TextRange::new(start+proposal.start,start+proposal.end);
                if range.start>offset||offset>range.end {continue;}
                exclusive|=batch.output.exclusive&&!batch.output.inferred;incomplete|=!batch.output.complete||batch.output.inferred;
                let prefix=&self.source[range.start..offset];let suffix=&self.source[offset..range.end];
                if !proposal.filter_text.starts_with(prefix)||!proposal.filter_text.ends_with(suffix){continue;}
                let identity=format!("editor-query:{}:{}",batch.owner,proposal.id);
                if !seen.insert(identity.clone()) {continue;}
                let insertion=if snippets {proposal.snippet.as_ref().unwrap_or(&proposal.insert_text)}else{&proposal.insert_text};
                let label=if batch.output.inferred&&!proposal.label.starts_with("[inferred] "){format!("[inferred] {}",proposal.label)}else{proposal.label.clone()};
                let mut item=self.item(&label,insertion,&proposal.detail,&identity,CompletionItemKind::METHOD,range,snippets&&proposal.snippet.is_some())?;
                item.filter_text=Some(proposal.filter_text.clone());
                item.sort_text=Some(format!("{:04}",items.len()));
                item.data=Some(serde_json::json!({"syntaxIdentity":identity,"addonProposal":{"owner":batch.owner,"id":proposal.id,"contextVerified":false,"queryContextVerified":true,"semanticGuarantee":false,"certainty":if batch.output.inferred {"inferred"}else{"unverified"},"source":"live-prefix-editor-query"}}));
                item.documentation=Some(super::super::hover_text::markup(super::super::hover_text::render_cached("Member proposal",&[
                    ("Provider",batch.owner.clone()),("Status","Provider queried in the current context; candidate semantics are unverified. Runtime access, overload selection and execution are unverified.".into()),
                    ("Description",proposal.documentation.clone())],None,&self.index.documentation_format),&self.index.documentation_format));
                items.push(item);if items.len()==MAX_ITEMS{incomplete=true;break 'batches;}
            }
        }
        Ok((exclusive||!items.is_empty()).then_some((CompletionList {is_incomplete:incomplete,items},exclusive)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{MappedSpan,SourceOrigin,OriginKind,TextRange as WireRange};
    fn document(source:&str,start:usize,end:usize)->Document{
        let mut d=Document::test_fixture(Arc::from(source));let code=source.find("    ").unwrap()+4;let line=&source[code..source.trim_end().len()];
        d.editor_queries.push(parser_wasm::host::EditorCompletionBatch {inputs:Vec::new(),owner:"test.member".into(),attempt:1,input:line.into(),span:MappedSpan {virtual_range:WireRange {start:code as u64,end:(code+line.len())as u64},origins:vec![SourceOrigin {kind:OriginKind::Exact,original_range:WireRange {start:code as u64,end:(code+line.len())as u64},expansion:None}]},output:parser_wasm::host::EditorCompletionOutput {typed_inputs:Vec::new(),version:1,complete:true,exclusive:true,inferred:false,proposals:vec![parser_wasm::host::EditorCompletionProposal {id:"getName".into(),label:"getName()".into(),filter_text:"getName".into(),insert_text:"getName()".into(),snippet:None,start,end,detail:"owned declaration".into(),documentation:String::new()}]}});d
    }
    #[test]fn query_edits_are_current_utf16_and_do_not_replace_receiver(){
        let source="on join:\n    set {_é} to player.getN\n";let code=source.find("set").unwrap();let start=source.find("getN").unwrap();let d=document(source,start-code,start+4-code);
        let list=d.at(d.lines.position(source,start+4).unwrap(),false).unwrap();assert_eq!(list.items.len(),1);
        let Some(CompletionTextEdit::Edit(edit))=&list.items[0].text_edit else{panic!()};assert_eq!(edit.new_text,"getName()");assert_eq!(edit.range.start,d.lines.position(source,start).unwrap());
        let mut changed=d;changed.source=Arc::from(source.replace("player.getN","console.getN"));assert!(!changed.at(changed.lines.position(&changed.source,start+4).unwrap(),false).unwrap().items.iter().any(|i|i.label=="getName()"));
    }
    #[test]fn several_providers_share_the_same_hard_menu_limit(){
        let source="on join:\n    player.\n";let mut d=document(source,7,7);
        let prototype=d.editor_queries[0].output.proposals[0].clone();
        d.editor_queries[0].output.proposals=(0..256).map(|i|{let mut p=prototype.clone();p.id=format!("a{i}");p}).collect();
        let mut second=d.editor_queries[0].clone();second.owner="other".into();
        let mut third=second.clone();third.owner="third".into();d.editor_queries.extend([second,third]);
        let list=d.at(d.lines.position(source,source.find("player.").unwrap()+7).unwrap(),false).unwrap();
        assert_eq!(list.items.len(),MAX_ITEMS);assert!(list.is_incomplete);
    }

    #[test]fn inferred_proposals_merge_with_native_candidates_and_keep_the_shared_bound(){
        let source="on join:\n    getN\n";let code=source.find("getN").unwrap();
        let mut d=document(source,0,4);
        let index=Arc::get_mut(&mut d.index).unwrap();let template=&mut index.templates[0];
        template.kind=SyntaxKind::Effect;template.label="getNative()".into();template.leading="getNative()".into();
        template.folded="getnative()".into();template.parts=vec![Part::Text("getNative()".into())];
        d.editor_queries[0].output.exclusive=false;d.editor_queries[0].output.inferred=true;d.editor_queries[0].output.complete=false;
        let list=d.at(d.lines.position(source,code+4).unwrap(),false).unwrap();
        assert!(list.items.iter().any(|i|i.label=="getNative()"),"ordinary candidates must survive");
        let proposed=list.items.iter().find(|i|i.filter_text.as_deref()==Some("getName")).unwrap();
        assert!(proposed.label.starts_with("[inferred] "));
        assert_eq!(proposed.data.as_ref().unwrap()["addonProposal"]["certainty"],"inferred");
        assert!(list.is_incomplete&&list.items.len()<=MAX_ITEMS);
    }

}
