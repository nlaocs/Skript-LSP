//! Data-only, current-document editing proposals from accepted root declarations.
//! These never become parser success, candidate-type witnesses or diagnostics.
use super::*;
use serde::Deserialize;

const MAX_EXPORT_BYTES: usize = 128 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXPORTS: usize = 32;
const MAX_CANDIDATES: usize = 256;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Export { version: u32, complete: bool, candidates: Vec<Candidate> }
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Candidate {
    id: String,
    label: String,
    insert_text: String,
    kind: Kind,
    case_sensitive: bool,
    detail: Option<String>,
    documentation: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind { Value, Expression }

#[derive(Default)]
pub(super) struct Snapshot { candidates: Vec<(String, Candidate)>, limited: bool }

pub(super) fn collect_or_suppress(result:&DocumentParseResult,source:&str,context:&BTreeMap<u32,Line>,lines:&LineIndex,blocked:&[TextRange]) -> Snapshot {
    match collect(result,source,context,lines,blocked) {
        Ok(snapshot)=>snapshot,
        Err(reason)=>{
            eprintln!("Ignoring invalid addon completion exports: {reason}");
            Snapshot {candidates:Vec::new(),limited:true}
        }
    }
}

pub(super) fn collect(
    result: &DocumentParseResult,
    source: &str,
    context: &BTreeMap<u32, Line>,
    lines: &LineIndex,
    blocked: &[TextRange],
) -> Result<Snapshot, String> {
    let nodes = result.ast.nodes.iter().map(|n| (n.id,n)).collect::<HashMap<_,_>>();
    let witnesses = result.semantic_witnesses();
    let mut output = Snapshot::default();
    let mut seen = HashMap::new();
    let mut bytes = 0usize;
    let mut exports = 0;
    // A public v1 export has whole-document scope and therefore requires a
    // selected physical root Structure. Child/foreign/unverified nodes cannot
    // publish globally visible names by copying the metadata key.
    let mut pending=result.ast.roots.clone();
    let mut visited=HashSet::new();
    while let Some(id)=pending.pop() {
        if !visited.insert(id) { continue; }
        let Some(node) = nodes.get(&id) else { return Err("missing addon completion root".into()); };
        pending.extend(node.children.iter().copied());
        if node.kind != parser_wasm::host::SyntaxKind::Structure { continue; }
        let Some(header) = exact_node(node,source).filter(|h| !blocked.iter().any(|b| overlaps(*h,*b))) else { continue; };
        let row = lines.position(source,header.start)?.line;
        if !context.get(&row).is_some_and(|line| line.root && !line.blocked && header.end <= line.range.end) { continue; }
        let mut owners = HashSet::new();
        for entry in node.metadata.iter().filter(|e| e.key == parser_wasm::DOCUMENT_COMPLETIONS_METADATA) {
            let Some(owner) = witnesses.metadata_owner(node,entry) else { continue; };
            exports += 1; bytes = bytes.saturating_add(entry.value.len());
            if exports > MAX_EXPORTS || bytes > MAX_TOTAL_BYTES || entry.value.len() > MAX_EXPORT_BYTES
                || !owners.insert(owner) { return Err("addon completion export exceeds bounds or duplicates owner".into()); }
            let export: Export = serde_json::from_str(&entry.value).map_err(|_| "invalid addon completion export")?;
            if export.version != 1 || export.candidates.len() > MAX_CANDIDATES { return Err("unsupported addon completion export version/bound".into()); }
            output.limited |= !export.complete;
            for candidate in export.candidates {
                if !valid(&candidate) { return Err("invalid addon completion proposal".into()); }
                let identity = (owner.to_owned(), candidate.id.clone());
                if let Some(previous)=seen.get(&identity) {
                    if previous != &candidate { return Err("conflicting addon completion identity".into()); }
                } else {
                    seen.insert(identity,candidate.clone());output.candidates.push((owner.to_owned(),candidate));
                }
            }
        }
    }
    Ok(output)
}

fn token(c: char) -> bool { c.is_alphanumeric() || matches!(c,'_'|'$') }
fn valid(c: &Candidate) -> bool {
    !c.id.is_empty() && c.id.len() <= 128 && !c.id.chars().any(char::is_control)
        && !c.label.is_empty() && c.label.len() <= 256 && !c.label.chars().any(char::is_control)
        && !c.insert_text.is_empty() && c.insert_text.len() <= 256 && c.insert_text.chars().all(token)
        && c.detail.as_ref().is_none_or(|s| s.len() <= 1024 && !s.contains('\0'))
        && c.documentation.as_ref().is_none_or(|s| s.len() <= 4096 && !s.contains('\0'))
}

impl Document {
    pub(super) fn push_provider_candidates(&self, line: &Line, offset: usize, pool: &mut ranking::Pool<'_>) -> Result<bool,String> {
        if line.root || !matches!(lexical_context(&self.source[line.code_start..offset]),Lexical::Code(_)) { return Ok(false); }
        let start=self.source[line.code_start..offset].char_indices().rev()
            .find(|(_,c)| !token(*c)).map_or(line.code_start,|(i,c)| line.code_start+i+c.len_utf8());
        let end=offset+self.source[offset..line.range.end].char_indices().find(|(_,c)| !token(*c))
            .map_or(line.range.end-offset,|(i,_)|i);
        let query=&self.source[start..offset];
        for (owner,candidate) in &self.provider_completions.candidates {
            let matches=if candidate.case_sensitive { candidate.insert_text.starts_with(query) } else { starts(&candidate.insert_text,query) };
            if !matches || !candidate.insert_text.ends_with(&self.source[offset..end])
                || !pool.may_accept_matching(&candidate.insert_text,&candidate.label,start) { continue; }
            let identity=format!("addon-proposal:{owner}:{}",candidate.id);
            let mut item=self.item(&candidate.label,&candidate.insert_text,candidate.detail.as_deref().unwrap_or("Addon editing proposal"),
                &identity,match candidate.kind { Kind::Value=>CompletionItemKind::VALUE,Kind::Expression=>CompletionItemKind::PROPERTY },
                TextRange::new(start,end),false)?;
            item.data=Some(serde_json::json!({"syntaxIdentity":identity,"addonProposal":{"owner":owner,"id":candidate.id,
                "contextVerified":false,"semanticGuarantee":false}}));
            let mut fields=vec![("Provider",owner.clone()),("Status","Editing proposal; parse and context are unverified".into())];
            if let Some(text)=&candidate.documentation { fields.push(("Description",text.clone())); }
            item.documentation=Some(super::super::hover_text::markup(super::super::hover_text::render_cached(
                "Addon proposal",&fields,None,&self.index.documentation_format),&self.index.documentation_format));
            pool.push_matching(item,&candidate.insert_text);
        }
        Ok(self.provider_completions.limited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(source:&str) -> Document {
        let mut document=Document::test_fixture(Arc::from(source));
        document.provider_completions.candidates.push(("test.provider".into(),Candidate {
            id:"class-alias".into(),label:"JavaString".into(),insert_text:"JavaString".into(),
            kind:Kind::Value,case_sensitive:true,detail:None,documentation:None}));
        document
    }
    #[test] fn proposals_use_utf16_prefix_and_existing_suffix_replacement() {
        let document=fixture("on load:\n    send \"😀\" and JavaString\n");
        let source=document.source.as_ref();let offset=source.find("JavaString").unwrap()+7;
        let position=document.lines.position(source,offset).unwrap();
        let list=document.at(position,false).unwrap();
        let item=list.items.iter().find(|i|i.label=="JavaString").unwrap();
        assert_eq!(item.insert_text_format,Some(InsertTextFormat::PLAIN_TEXT));
        assert_eq!(item.data.as_ref().unwrap()["addonProposal"]["contextVerified"],false);
        let Some(CompletionTextEdit::Edit(edit))=&item.text_edit else {panic!("plain edit")};
        assert_eq!(edit.new_text,"JavaStr");
        assert_eq!(edit.range.start,document.lines.position(source,offset-7).unwrap());
        assert_eq!(edit.range.end,position);
        let mut applied=source.to_owned();let start=document.lines.offset(source,edit.range.start).unwrap();
        applied.replace_range(start..offset,&edit.new_text);assert_eq!(applied,source);
    }
    #[test] fn proposals_do_not_escape_lexical_context_or_persist_in_a_new_document() {
        for source in ["JavaStr", "on load:\n    send \"JavaStr", "on load:\n    # JavaStr", "on load:\n    send {JavaStr}", "on load:\n    send javaStr"] {
            let document=fixture(source);let offset=source.find(if source.contains("javaStr") {"javaStr"}else{"JavaStr"}).unwrap()+7;
            let list=document.at(document.lines.position(source,offset).unwrap(),false).unwrap();
            assert!(!list.items.iter().any(|i|i.label=="JavaString"),"{source}");
        }
        let source="on load:\n    send JavaStr";let document=Document::test_fixture(Arc::from(source));
        assert!(!document.at(document.lines.position(source,source.len()).unwrap(),false).unwrap().items.iter().any(|i|i.label=="JavaString"));
    }
    #[test] fn export_is_untyped_plain_identifier_data() {
        let value=serde_json::json!({"version":1,"complete":true,"candidates":[{"id":"alias","label":"JavaString",
            "insertText":"JavaString","kind":"value","caseSensitive":true,"detail":null,"documentation":null}]});
        let export:Export=serde_json::from_value(value.clone()).unwrap();assert!(valid(&export.candidates[0]));
        let mut wrong=value;wrong["candidates"][0]["returnType"]="java.lang.String".into();
        assert!(serde_json::from_value::<Export>(wrong).is_err());
    }
    #[test] fn copied_foreign_metadata_cannot_publish_names_and_changed_forests_lose_authority() {
        let source="on load:\n    send 1 to console\n";
        let catalog=crate::lsp::test_support::runtime_catalog();
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog),..Default::default()}).unwrap();
        let mut result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///completion-exports","file:///completion-exports/test.sk",1,source.to_owned()),Default::default()).unwrap();
        let node=result.ast.nodes.iter_mut().find(|n|n.kind==parser_wasm::host::SyntaxKind::Structure).unwrap();
        node.metadata.push(parser_wasm::bindings::nlaocs::skript_parser_addon::types::MetadataEntry {key:parser_wasm::DOCUMENT_COMPLETIONS_METADATA.into(),owner_component_id:Some("foreign.provider".into()),
            value:serde_json::json!({"version":1,"complete":true,"candidates":[{"id":"forged","label":"ForgedAlias","insertText":"ForgedAlias","kind":"value","caseSensitive":true}]}).to_string()});
        let lines=LineIndex::new(source);let context=raw_context(&result.raw_tree,source,&lines,true).unwrap();
        assert!(collect(&result,source,&context,&lines,&[]).unwrap().candidates.is_empty());
        result.ast.roots.push(u64::MAX);
        let suppressed=collect_or_suppress(&result,source,&context,&lines,&[]);
        assert!(suppressed.candidates.is_empty()&&suppressed.limited);
        let mut document=Document::test_fixture(Arc::from("on load:\n    send JavaStr"));
        document.provider_completions=suppressed;
        assert!(document.at(document.lines.position(&document.source,document.source.len()).unwrap(),false).is_ok());
    }
    #[test] fn edits_cannot_inject_snippets_newlines_or_multiple_tokens() {
        let mut candidate:Candidate=serde_json::from_value(serde_json::json!({"id":"id","label":"x","insertText":"x",
            "kind":"value","caseSensitive":true,"detail":null,"documentation":null})).unwrap();
        for text in ["x\ny","x y","${1:world}","x.y",""] {candidate.insert_text=text.into();assert!(!valid(&candidate));}
        candidate.insert_text="$日本語".into();assert!(valid(&candidate));
    }
}
