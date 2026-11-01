//! Bounded, read-only native capture headers for the current semantic call.
//! This is neither an importable parse arena nor a value/effect certificate.
use std::{collections::BTreeSet, io::{self, Write}};
use serde::Serialize;
use skript_parser::{ExpressionListConjunction, ExpressionNode, ExpressionNodeKind, MappedSpan,
    OriginKind, ParsedCapture, ParsedCaptureStatus, ParsedCaptureValue};
use syntaxes::{Multiplicity, PossibleReturnTypesState};
use super::WitMetadataEntry;

pub(super) const CAPABILITY: &str = crate::CAPABILITY_NATIVE_EXPRESSION_INPUTS;
pub(super) const CONTEXT: &str = crate::REGISTERED_CONTEXT_NATIVE_EXPRESSION_INPUTS;
pub(super) const KEY: &str = crate::REGISTERED_METADATA_NATIVE_EXPRESSION_INPUTS;
const MAX_CAPTURES: usize = 64;
const MAX_CHILDREN: usize = 64;
const MAX_TYPES: usize = 64;
const MAX_NAME_BYTES: usize = 512;
const MAX_ORIGINS: usize = 32;
const MAX_BYTES: usize = 64 * 1024;

#[derive(Serialize)]
struct View<'a> { version: u32, captures: Vec<Capture<'a>> }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct Capture<'a> { capture_index: usize, root: Header<'a>, direct_children: Vec<Header<'a>> }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct Header<'a> {
    native_kind: &'static str, conjunction: Option<&'static str>, span: Span,
    return_type: Option<&'a str>, possible_return_types: Vec<&'a str>,
    possible_return_types_state: &'static str, multiplicity: Option<&'static str>,
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct Span { virtual_range: Range, origins: Vec<Origin> }
#[derive(Serialize)]
struct Range { start: usize, end: usize }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct Origin { original_range: Range, kind: &'static str, expansion: Option<u32> }

impl Span {
    fn from_native(span: &MappedSpan)->Option<Self> {
        if span.origins.len()>MAX_ORIGINS {return None;}
        Some(Self {virtual_range:Range {start:span.virtual_range.start,end:span.virtual_range.end},
            origins:span.origins.iter().map(|origin|Origin {
                original_range:Range {start:origin.original_range.start,end:origin.original_range.end},
                kind:match origin.kind {OriginKind::Exact=>"exact",OriginKind::Replaced=>"replaced",OriginKind::Anchored=>"anchored"},
                expansion:origin.expansion.map(|id|id.get()),
            }).collect()})
    }
}
impl<'a> Header<'a> {
    fn from_native(node:&'a ExpressionNode)->Option<Self> {
        if node.possible_return_types.len()>MAX_TYPES
            || node.return_type.as_ref().is_some_and(|t|t.as_str().len()>MAX_NAME_BYTES)
            || node.possible_return_types.iter().any(|t|t.as_str().len()>MAX_NAME_BYTES) {return None;}
        let (native_kind,conjunction)=match node.kind {
            ExpressionNodeKind::List {conjunction} => ("list",Some(match conjunction {
                ExpressionListConjunction::And=>"and",ExpressionListConjunction::Or=>"or"})),
            ExpressionNodeKind::Grouped => ("grouped",None),
            ExpressionNodeKind::Default {..} => ("default",None),
            ExpressionNodeKind::Registered {..} => ("registered",None),
            ExpressionNodeKind::Variable {..} => ("variable",None),
            ExpressionNodeKind::Literal {..} => ("literal",None),
            ExpressionNodeKind::Function {..} => ("function",None),
            ExpressionNodeKind::NativeFunction {..} => ("native-function",None),
            ExpressionNodeKind::Arithmetic {..} => ("arithmetic",None),
            ExpressionNodeKind::Custom {..} => ("custom",None),
        };
        Some(Self {native_kind,conjunction,span:Span::from_native(&node.span.mapped)?,
            return_type:node.return_type.as_ref().map(|t|t.as_str()),
            possible_return_types:node.possible_return_types.iter().map(|t|t.as_str()).collect(),
            possible_return_types_state:match node.possible_return_types_state {
                PossibleReturnTypesState::Complete=>"complete",PossibleReturnTypesState::Partial=>"partial",
                PossibleReturnTypesState::Unresolved=>"unresolved"},
            multiplicity:node.multiplicity.map(|m|match m {
                Multiplicity::Single=>"single",Multiplicity::Multiple=>"multiple",Multiplicity::Both=>"both"}),
        })
    }
}

struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self,bytes:&[u8])->io::Result<usize> {
        if bytes.len()>MAX_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("native expression input projection exceeds 64 KiB"));
        }
        self.0.extend_from_slice(bytes);Ok(bytes.len())
    }
    fn flush(&mut self)->io::Result<()> {Ok(())}
}

pub(super) fn metadata(captures:&[ParsedCapture])->Option<WitMetadataEntry> {
    if captures.len()>MAX_CAPTURES {return None;}
    let mut seen=BTreeSet::new();let mut total_children=0usize;let mut entries=Vec::new();
    for capture in captures {
        if !seen.insert(capture.capture_index) {return None;}
        if capture.binding.parser_id!=skript_parser::HOST_EXPRESSION_PARSER_ID
            || capture.result.parser_id!=skript_parser::HOST_EXPRESSION_PARSER_ID
            || capture.result.status!=ParsedCaptureStatus::Success {continue;}
        let Some(ParsedCaptureValue::Expression(node))=&capture.result.value else {continue;};
        if capture.result.span.mapped!=node.span.mapped {return None;}
        total_children=total_children.checked_add(node.children.len())?;
        if total_children>MAX_CHILDREN {return None;}
        let root=Header::from_native(node)?;
        let direct_children=node.children.iter().map(Header::from_native).collect::<Option<Vec<_>>>()?;
        entries.push(Capture {capture_index:capture.capture_index,root,direct_children});
    }
    let mut bytes=Bounded(Vec::new());
    serde_json::to_writer(&mut bytes,&View {version:1,captures:entries}).ok()?;
    Some(WitMetadataEntry {key:KEY.into(),owner_component_id:None,value:String::from_utf8(bytes.0).ok()?})
}

pub(super) fn same(left:&[WitMetadataEntry],right:&[WitMetadataEntry])->bool {
    fn values(items:&[WitMetadataEntry])->impl Iterator<Item=&str> {
        items.iter().filter(|m|m.key==KEY&&m.owner_component_id.is_none()).map(|m|m.value.as_str())
    }
    values(left).eq(values(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skript_parser::{ExpressionNode,MatchSpan,MappedSource,RegisteredCaptureBinding,ParsedCaptureResult,TextRange};
    use std::collections::BTreeMap;
    use syntaxes::ClassName;
    fn node(text:&str)->ExpressionNode {
        let source=MappedSource::identity(text);let range=TextRange::new(0,text.len());
        static LEAF: std::sync::LazyLock<ExpressionNode> = std::sync::LazyLock::new(||
            super::super::capture_view_tests::fixture().1);
        let mut node=LEAF.clone();node.effects=None;
        node.kind=ExpressionNodeKind::Literal {parser_id:"test".into()};node.function=None;
        node.span=MatchSpan {local_range:range,mapped:source.map_range(range).unwrap()};
        node.return_type=Some(ClassName("java.lang.String".into()));
        node.possible_return_types=vec![ClassName("java.lang.String".into())];
        node.possible_return_types_state=PossibleReturnTypesState::Complete;
        node.multiplicity=Some(Multiplicity::Single);node.children.clear();node.captures.clear();
        node.tags.clear();node.public_data.clear();node.metadata.clear();node
    }
    fn capture(root:ExpressionNode)->ParsedCapture {
        ParsedCapture {capture_index:2,binding:RegisteredCaptureBinding {capture_index:2,parser_id:"host.expression".into(),required:true,options:BTreeMap::new()},
            result:ParsedCaptureResult {parser_id:"host.expression".into(),status:ParsedCaptureStatus::Success,
                span:root.span.clone(),summary:None,value:Some(ParsedCaptureValue::Expression(root)),diagnostics:vec![],attachments:vec![]}}
    }
    #[test]
    fn native_expression_inputs_distinguish_conjunction_and_preserve_order_nullability_and_spans(){
        for (mode,expected) in [(ExpressionListConjunction::And,"and"),(ExpressionListConjunction::Or,"or")] {
            let mut root=node("🦀 args");root.kind=ExpressionNodeKind::List {conjunction:mode};root.multiplicity=Some(Multiplicity::Multiple);
            let mut unknown=node("second");unknown.return_type=None;unknown.possible_return_types.clear();unknown.possible_return_types_state=PossibleReturnTypesState::Unresolved;
            root.children=vec![node("first"),unknown];
            let expected_end=root.span.mapped.virtual_range.end;
            let data:serde_json::Value=serde_json::from_str(&metadata(&[capture(root)]).unwrap().value).unwrap();
            assert_eq!(data["captures"][0]["root"]["nativeKind"],"list");
            assert_eq!(data["captures"][0]["root"]["conjunction"],expected);
            assert_eq!(data["captures"][0]["root"]["span"]["virtualRange"]["end"],expected_end);
            assert_eq!(data["captures"][0]["directChildren"][0]["returnType"],"java.lang.String");
            assert!(data["captures"][0]["directChildren"][1]["returnType"].is_null());
            assert_eq!(data["captures"][0]["directChildren"].as_array().unwrap().len(),2);
        }
    }
    #[test]
    fn native_expression_inputs_reject_ambiguity_and_limits_without_truncation(){
        let one=capture(node("x"));assert!(metadata(&[one.clone(),one.clone()]).is_none());
        let mut wide=node("args");wide.children=vec![node("a");65];assert!(metadata(&[capture(wide)]).is_none());
        let mut long=node("long");long.return_type=Some(ClassName("X".repeat(513)));assert!(metadata(&[capture(long)]).is_none());
        let mut many=node("many");many.possible_return_types=vec![ClassName("x".into());65];assert!(metadata(&[capture(many)]).is_none());
        let mut large=node("large");let mut child=node("child");child.possible_return_types=vec![ClassName("x".repeat(64));64];
        large.children=vec![child;64];assert!(metadata(&[capture(large)]).is_none(),"valid bounded headers must still obey the final encoded byte limit");
        let mut foreign=one;foreign.result.span.mapped.virtual_range.end+=1;assert!(metadata(&[foreign]).is_none());
    }
    #[test]
    fn native_expression_inputs_are_immutable_and_owned_lookalikes_are_not_native(){
        let entry=metadata(&[capture(node("x"))]).unwrap();assert!(same(std::slice::from_ref(&entry),std::slice::from_ref(&entry)));
        assert!(!same(std::slice::from_ref(&entry),&[]));
        let mut changed=entry.clone();changed.value.push(' ');assert!(!same(&[entry],&[changed]));
        let mut owned=metadata(&[capture(node("x"))]).unwrap();owned.owner_component_id=Some("foreign".into());
        assert!(same(&[],&[owned]));
    }

    fn manifest(kind:crate::bindings::nlaocs::skript_parser_addon::types::SyntaxKind)
        ->crate::bindings::nlaocs::skript_parser_addon::types::ComponentManifest {
        use crate::bindings::nlaocs::skript_parser_addon::types::*;
        let (phase,capability)=match kind {SyntaxKind::Expression=>(HookPhase::Expression,"parser.expression"),
            SyntaxKind::Condition=>(HookPhase::Condition,"parser.condition"),_=>panic!()};
        ComponentManifest {component_id:"test.native-bindings".into(),component_version:"1".into(),
            abi:AbiVersion {major:crate::ABI_VERSION.major,minor:crate::ABI_VERSION.minor},
            capabilities:vec![CapabilityRequirement {id:capability.into(),minimum_version:1,required:true}],
            subscriptions:vec![HookSubscription {id:"test.native".into(),target:HookTarget::SyntaxKind(kind),phase,
                priority:0,mode:HookMode::Transform,capability_id:capability.into(),selector:HookSelector {
                    pattern_index:None,pattern_source:None,mark:None,tags:vec![],captures:vec![],
                    return_type:None,multiplicity:None,metadata:vec![]}}],
            registered_syntax_handlers:vec![RegisteredSyntaxHandler {handler_id:"test.native".into(),kind,phase,
                targets:vec![RegisteredSyntaxHandlerTarget::ClassSuffix(".Native".into())],pattern_indices:vec![],
                pattern_sources:vec![],required_tags:vec![],forbidden_tags:vec![],marks:vec![],capture_parsers:vec![],
                context_requirements:vec![]}],catalog_annotations:vec![],state_namespaces:vec![],
            incremental_local_subscriptions:vec![],incremental_header_handlers:vec![],
            incremental_body_subscriptions:vec![],incremental_body_handler_subscriptions:vec![]}
    }
    #[test]
    fn native_expression_inputs_context_is_explicit_and_old_hosts_reject_it(){
        use crate::bindings::nlaocs::skript_parser_addon::types::*;
        let mut m=manifest(SyntaxKind::Expression);
        m.registered_syntax_handlers[0].context_requirements=vec![CONTEXT.into()];
        let caps=super::super::host_capabilities();
        assert!(super::super::validate_manifest(&m,&caps).is_err());
        m.capabilities.push(CapabilityRequirement {id:CAPABILITY.into(),minimum_version:1,required:true});
        super::super::validate_manifest(&m,&caps).unwrap();
        let old=caps.into_iter().filter(|c|c.id!=CAPABILITY).collect::<Vec<_>>();
        assert!(super::super::validate_manifest(&m,&old).is_err());
        m.capabilities.last_mut().unwrap().required=false;
        assert!(super::super::validate_manifest(&m,&super::super::host_capabilities()).is_err());
        m.registered_syntax_handlers[0].kind=SyntaxKind::Condition;
        assert!(super::super::validate_manifest(&m,&super::super::host_capabilities()).is_err());
    }
    #[test]
    fn native_condition_expression_capture_requires_v2_and_restores_v1_without_binding(){
        use crate::bindings::nlaocs::skript_parser_addon::types::*;
        let mut m=manifest(SyntaxKind::Condition);let caps=super::super::host_capabilities();
        super::super::validate_manifest(&m,&caps).unwrap();
        m.registered_syntax_handlers[0].capture_parsers=vec![CaptureParserBinding {capture_index:0,
            parser_id:"host.expression".into(),required:true,options:vec![]}];
        assert!(super::super::validate_manifest(&m,&caps).is_err());
        m.capabilities[0].minimum_version=2;super::super::validate_manifest(&m,&caps).unwrap();
        let mut old=caps.clone();old.iter_mut().find(|c|c.id=="parser.condition").unwrap().version=1;
        assert!(super::super::validate_manifest(&m,&old).is_err());
        m.capabilities[0].required=false;assert!(super::super::validate_manifest(&m,&caps).is_err());
    }
}
