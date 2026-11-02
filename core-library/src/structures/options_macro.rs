use std::collections::{HashMap, HashSet};

use crate::nlaocs::skript_parser_addon::{tree_data, types::{
    AddonError, AddonErrorKind, Diagnostic, DiagnosticSeverity, GeneratedRawNode, MacroUnavailability,
    GeneratedRawNodeKind, GeneratedRawTree, HookDecision, HookEffects, NodePage,
    NodeHeader, NodeSpanKind, NodeSummary, RawNodeKind, ReplaceNodeEdit, RetainedChildren,
    RetainedChildrenPlacement, TextEdit, TextRange, TreeEdit, TreeError, TreeMacroInput, TreeMacroOutput, TreeReference,
}};

fn access_error(error: TreeError) -> AddonError {
    AddonError { kind: AddonErrorKind::Internal,
        message: format!("Tree view read failed: {error:?}"), diagnostics: Vec::new() }
}
fn invalid(message: &str) -> AddonError {
    AddonError { kind: AddonErrorKind::InvalidPayload, message: message.into(), diagnostics: Vec::new() }
}

fn pages(mut query: impl FnMut(u64) -> Result<NodePage, AddonError>) -> Result<Vec<NodeHeader>, AddonError> {
    let mut cursor = 0;
    let mut nodes = Vec::new();
    loop {
        let page = query(cursor)?;
        nodes.extend(page.nodes);
        match page.next {
            Some(next) if next > cursor => cursor = next,
            Some(_) => return Err(invalid("Tree page cursor did not advance")),
            None => return Ok(nodes),
        }
    }
}

pub(crate) fn expand(input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
    // The target header is constant-sized. Comments/generated nodes need no
    // owned tree, line, child list or origin vectors merely to decline expansion.
    if !matches!(input.target.kind, RawNodeKind::Simple | RawNodeKind::Section)
    {
        return Ok(unchanged(Vec::new()));
    }
    let target = tree_data::node(input.tree, input.target.id).map_err(access_error)?
        .ok_or_else(|| invalid("Tree macro target is absent from its active view"))?;
    // Option lookup is read-only. With no marker, no option can change this
    // target or emit a missing-option diagnostic, including the preload pass.
    if !target.text.contains("{@") || inside_options(&input.tree, &target)? {
        return Ok(unchanged(Vec::new()));
    }
    let unavailable=unavailable_bindings(input.tree)?;
    if let Some(dependency)=unavailable.used_by(&target.text) {return Ok(unevaluated(dependency));}
    // Hygiene still forbids positive expansion in a foreign generated context.
    // Negative unavailability is checked first: generated quoted markers cannot
    // become known literals merely by crossing a macro boundary.
    if input.target.syntax_context!=0 {return Ok(unchanged(Vec::new()));}
    let root_headers = pages(|cursor| tree_data::roots(input.tree, cursor, 128).map_err(access_error))?;
    let mut roots = Vec::with_capacity(root_headers.len());
    for header in root_headers {
        if matches!(header.kind, RawNodeKind::Section) {
            roots.push(tree_data::node(input.tree, header.id).map_err(access_error)?
                .ok_or_else(|| invalid("Root section is absent from its Tree view"))?);
        } else {
            // Root order still determines first-pass option visibility. No
            // comment/blank/simple text is needed to recognize an options root.
            roots.push(NodeSummary { header, text: String::new() });
        }
    }
    let all_options = collect_options(&input.tree, &roots, None)?;
    let earlier;
    let options = if target.header.parent.is_none() {
        earlier = collect_options(&input.tree, &roots, Some(target.header.id))?;
        &earlier
    } else {
        &all_options
    };
    let first = replace_options(&target.text, options);
    if let Some(dependency)=unavailable.used_by(&first.text) {return Ok(unevaluated(dependency));}
    let mut text = first.text;
    let mut missing = first.missing;
    let mut edits = Some(first.edits);
    // Preserve the original single-pass and command/function preload behavior.
    // Generated nodes still re-enter the normal host macro pipeline.
    if target.header.parent.is_none() && preload_replaces_options(&text) {
        let second = replace_options(&text, &all_options);
        if let Some(dependency)=unavailable.used_by(&second.text) {return Ok(unevaluated(dependency));}
        // Second-pass substitutions can cross a generated/retained boundary.
        // Keep the established generated replacement until composition of that
        // case has an explicit source witness; never guess first-pass offsets.
        if second.text != text { edits = None; }
        text = second.text;
        missing.extend(second.missing);
    }
    let diagnostic_span = if missing.is_empty() { None } else {
        Some(match tree_data::span(input.tree, target.header.id, NodeSpanKind::Code).map_err(access_error)? {
            Some(span) => span,
            None => tree_data::span(input.tree, target.header.id, NodeSpanKind::Node).map_err(access_error)?
                .ok_or_else(|| invalid("Tree macro target has no mapped source span"))?,
        })
    };
    let diagnostics = missing.into_iter().map(|name| Diagnostic {
        code: "core.options.undefined".into(), message: format!("undefined option {{@{name}}}"),
        severity: DiagnosticSeverity::Error, span: diagnostic_span.as_ref().unwrap().clone(), related: Vec::new(),
    }).collect();
    if text == target.text { return Ok(unchanged(diagnostics)); }
    if let Some(edits) = edits
        && edit_code_supported(&input.tree, &target)?
    {
        return Ok(TreeMacroOutput { unavailable: Vec::new(),
            decision: HookDecision::ContinueProcessing,
            edit: Some(TreeEdit::EditCode(edits)), effects: effects(diagnostics),
        });
    }
    let (kind, retained_children) = match target.header.kind {
        RawNodeKind::Simple => (GeneratedRawNodeKind::Simple, None),
        RawNodeKind::Section => (GeneratedRawNodeKind::Section, Some(RetainedChildren {
            target: 0, placement: RetainedChildrenPlacement::Append,
        })),
        _ => return Ok(unchanged(diagnostics)),
    };
    Ok(TreeMacroOutput { unavailable: Vec::new(), decision: HookDecision::ContinueProcessing,
        edit: Some(TreeEdit::ReplaceNode(ReplaceNodeEdit {
            replacement: GeneratedRawTree { roots: vec![0], nodes: vec![GeneratedRawNode {
                id: 0, kind, text, children: Vec::new(),
            }] }, retained_children,
        })), effects: effects(diagnostics),
    })
}

const OPTION_NAMESPACE:&str="skript.options";
struct UnavailableOptions {complete:bool,names:HashMap<String,u64>}
impl UnavailableOptions {
    fn used_by(&self,text:&str)->Option<MacroUnavailability> {
        let mut rest=text;
        while let Some(start)=rest.find("{@") {
            let marker=&rest[start+2..];let end=marker.find('}')?;
            if !self.complete {return Some(MacroUnavailability::IncompleteNamespace(OPTION_NAMESPACE.into()));}
            if let Some(id)=self.names.get(&marker[..end]) {return Some(MacroUnavailability::Binding(*id));}
            rest=&marker[end+1..];
        }
        None
    }
}
fn unavailable_bindings(tree:TreeReference)->Result<UnavailableOptions,AddonError> {
    let mut cursor=0;let mut complete=true;let mut names=HashMap::new();
    loop {
        let page=tree_data::unavailable_bindings(tree,OPTION_NAMESPACE,cursor,128).map_err(access_error)?;
        complete&=page.complete;
        for binding in page.bindings {
            if names.len()>=256||names.insert(binding.name,binding.id).is_some(){return Err(invalid("Unavailable options page count/identity mismatch"));}
        }
        match page.next {Some(next) if next>cursor=>cursor=next,None=>return Ok(UnavailableOptions {complete,names}),_=>return Err(invalid("Unavailable options cursor did not advance"))}
    }
}
fn unevaluated(dependency:MacroUnavailability)->TreeMacroOutput {
    TreeMacroOutput {decision:HookDecision::ContinueProcessing,edit:None,unavailable:vec![dependency],effects:effects(Vec::new())}
}

fn edit_code_supported(tree: &TreeReference, target: &NodeSummary) -> Result<bool, AddonError> {
    let details = match tree_data::details(*tree, target.header.id) {
        Ok(Some(details)) => details,
        Err(TreeError::ByteQuota | TreeError::QueryQuota) => return Ok(false),
        Err(error) => return Err(access_error(error)),
        Ok(None) => return Err(invalid("Tree macro target disappeared from its active view")),
    };
    let Some(code) = details.code_span else { return Ok(false); };
    let Some(start) = code.virtual_range.start.checked_sub(details.line.content_span.virtual_range.start)
        .and_then(|at| usize::try_from(at).ok()) else { return Ok(false); };
    let Some(end) = code.virtual_range.end.checked_sub(details.line.content_span.virtual_range.start)
        .and_then(|at| usize::try_from(at).ok()) else { return Ok(false); };
    let physical = details.line.raw_text.get(start..end);
    let physical = if target.header.kind == RawNodeKind::Section {
        physical.and_then(|code| code.strip_suffix(':'))
    } else { physical };
    Ok(physical == Some(target.text.as_str()))
}

fn inside_options(tree: &TreeReference, target: &NodeSummary) -> Result<bool, AddonError> {
    if is_options_root(target) { return Ok(true); }
    let mut current = target.header.parent;
    let mut seen = HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id) { return Err(invalid("Tree view contains a parent cycle")); }
        let node = tree_data::node(*tree, id).map_err(access_error)?
            .ok_or_else(|| invalid("Tree view parent is absent"))?;
        if is_options_root(&node) { return Ok(true); }
        current = node.header.parent;
    }
    Ok(false)
}

fn collect_options(tree: &TreeReference, roots: &[NodeSummary], stop_before: Option<u64>)
    -> Result<HashMap<String, String>, AddonError> {
    let mut options = HashMap::new();
    for root in roots {
        if Some(root.header.id) == stop_before { break; }
        if is_options_root(root) { collect_section(tree, root, "", &mut options)?; }
    }
    Ok(options)
}

fn collect_section(tree: &TreeReference, section: &NodeSummary, prefix: &str,
    options: &mut HashMap<String, String>) -> Result<(), AddonError> {
    let children = pages(|cursor| tree_data::children(*tree, section.header.id, cursor, 128)
        .map_err(access_error)?.ok_or_else(|| invalid("Options section is absent from its Tree view")))?;
    for header in children {
        if matches!(header.kind, RawNodeKind::Blank | RawNodeKind::Comment | RawNodeKind::Invalid) { continue; }
        let child = tree_data::node(*tree, header.id).map_err(access_error)?
            .ok_or_else(|| invalid("Options child is absent from its Tree view"))?;
        match child.header.kind {
            RawNodeKind::Simple => {
                if let Some((key, value)) = child.text.split_once(':') {
                    options.insert(format!("{prefix}{}", key.trim()), value.trim().to_owned());
                }
            }
            RawNodeKind::Section => {
                collect_section(tree, &child, &format!("{prefix}{}.", child.text.trim()), options)?;
            }
            RawNodeKind::Blank | RawNodeKind::Comment | RawNodeKind::Invalid => {}
        }
    }
    Ok(())
}

fn is_options_root(node: &NodeSummary) -> bool {
    node.header.parent.is_none() && matches!(node.header.kind, RawNodeKind::Section)
        && node.text.trim().eq_ignore_ascii_case("options")
}

fn preload_replaces_options(text: &str) -> bool {
    let text = text.trim_start();
    text.get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("command "))
        || text
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("function "))
}

struct Replacement {
    text: String,
    missing: Vec<String>,
    edits: Vec<TextEdit>,
}

fn replace_options(input: &str, options: &HashMap<String, String>) -> Replacement {
    let mut text = String::with_capacity(input.len());
    let mut missing = Vec::new();
    let mut edits = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("{@") {
        text.push_str(&rest[..start]);
        let marker = &rest[start..];
        let Some(end) = marker[2..].find('}') else {
            text.push_str(marker);
            rest = "";
            break;
        };
        let end = end + 2;
        let name = &marker[2..end];
        if let Some(value) = options.get(name) {
            text.push_str(value);
            if value != &marker[..=end] {
                let offset = input.len() - rest.len() + start;
                edits.push(TextEdit {
                    range: TextRange { start: offset as u64, end: (offset + end + 1) as u64 },
                    replacement: value.clone(), anchor: None,
                });
            }
        } else {
            text.push_str(&marker[..=end]);
            missing.push(name.to_owned());
        }
        rest = &marker[end + 1..];
    }
    text.push_str(rest);
    Replacement { text, missing, edits }
}

fn unchanged(diagnostics: Vec<Diagnostic>) -> TreeMacroOutput {
    TreeMacroOutput { unavailable: Vec::new(),
        decision: HookDecision::ContinueProcessing,
        edit: None,
        effects: effects(diagnostics),
    }
}

fn effects(diagnostics: Vec<Diagnostic>) -> HookEffects {
    HookEffects {
        diagnostics,
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
        parse_results: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_markers_are_not_constants_even_when_quoted_or_substituted() {
        let unavailable=UnavailableOptions {complete:true,names:HashMap::from([("computed".into(),9)])};
        for text in ["{@computed}","set {_x} to \"{@computed}\"","option {@computed}"] {
            assert!(matches!(unavailable.used_by(text),Some(MacroUnavailability::Binding(9))),"{text}");
        }
        assert!(unavailable.used_by("{@missing}").is_none());
        let first=replace_options("\"{@static}\"",&HashMap::from([("static".into(),"{@computed}".into())]));
        assert!(matches!(unavailable.used_by(&first.text),Some(MacroUnavailability::Binding(9))));
        let incomplete=UnavailableOptions {complete:false,names:HashMap::new()};
        assert!(matches!(incomplete.used_by("{@known}"),Some(MacroUnavailability::IncompleteNamespace(_))));
        assert!(incomplete.used_by("ordinary literal").is_none());
    }

    #[test]
    fn replacement_edits_use_original_utf8_byte_offsets() {
        let result=replace_options("\u{65e5}{@x}/{@x}", &HashMap::from([("x".into(),"longer".into())]));
        assert_eq!(result.text,"\u{65e5}longer/longer");
        assert!(result.missing.is_empty());
        assert_eq!(result.edits.len(),2);
        assert_eq!((result.edits[0].range.start,result.edits[0].range.end),(3,7));
        assert_eq!((result.edits[1].range.start,result.edits[1].range.end),(8,12));
        assert!(result.edits.iter().all(|edit|edit.anchor.is_none()));
    }

    #[test]
    fn replacement_is_single_pass_and_preserves_missing_markers() {
        let options = HashMap::from([
            ("first".to_owned(), "{@second}".to_owned()),
            ("second".to_owned(), "value".to_owned()),
        ]);
        let result = replace_options("{@first} {@missing}", &options);

        assert_eq!(result.text, "{@second} {@missing}");
        assert_eq!(result.missing, ["missing"]);
    }

    #[test]
    fn only_command_and_function_repeat_replacement_after_preload() {
        assert!(preload_replaces_options("command /test"));
        assert!(preload_replaces_options("FUNCTION test()"));
        assert!(!preload_replaces_options("on load"));
    }
}
