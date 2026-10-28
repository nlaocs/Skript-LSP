//! Isolated generic State-writer fixtures; no language/addon-specific semantics.
use crate::exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use crate::nlaocs::skript_parser_addon::{state_store, types::*};
use parser_wasm::{ABI_VERSION, CAPABILITY_HOOKS, CAPABILITY_STRUCTURE_PARSER, CAPABILITY_STATE_STORE};
#[cfg(not(feature="addon-b"))]
const ID:&str="test.semantic-state-a";
#[cfg(feature="addon-b")]
const ID:&str="test.semantic-state-b";
const NS:&str="semantic-state";
const SCHEMA:&str="test.semantic-state";
pub(crate) struct SemanticStateAddon;
fn effects()->HookEffects {HookEffects {diagnostics:vec![],context_updates:vec![],parse_requests:vec![],parse_results:vec![]}}
fn error(message:impl Into<String>)->AddonError {AddonError {kind:AddonErrorKind::InvalidPayload,message:message.into(),diagnostics:vec![]}}
impl addon::Guest for SemanticStateAddon {
    fn prepare_source_step(context: crate::nlaocs::skript_parser_addon::types::SourcePreparationContext)
        -> Result<crate::nlaocs::skript_parser_addon::types::SourcePreparationStatus, crate::nlaocs::skript_parser_addon::types::AddonError> {
        let _ = context; Ok(crate::nlaocs::skript_parser_addon::types::SourcePreparationStatus::Ready("unused".into()))
    }

    fn manifest()->ComponentManifest {
        ComponentManifest {component_id:ID.into(),component_version:env!("CARGO_PKG_VERSION").into(),
            abi:AbiVersion {major:ABI_VERSION.major,minor:ABI_VERSION.minor},
            capabilities:[CAPABILITY_HOOKS,CAPABILITY_STRUCTURE_PARSER,CAPABILITY_STATE_STORE].into_iter()
                .map(|id|CapabilityRequirement {id:id.into(),minimum_version:1,required:true}).collect(),
            incremental_local_subscriptions:vec![],incremental_header_handlers:vec![],
            incremental_body_subscriptions:vec![],incremental_body_handler_subscriptions:vec![],
            subscriptions:vec![HookSubscription {id:"state".into(),target:HookTarget::SyntaxKind(SyntaxKind::Structure),
                phase:HookPhase::Structure,priority:100,mode:HookMode::Observe,capability_id:CAPABILITY_STRUCTURE_PARSER.into(),
                selector:HookSelector {pattern_index:None,pattern_source:None,mark:None,tags:vec![],captures:vec![],return_type:None,multiplicity:None,metadata:vec![]}}],
            registered_syntax_handlers:vec![],catalog_annotations:vec![],state_namespaces:vec![
                StateNamespaceDeclaration {name:NS.into(),visibility:StateNamespaceVisibility::Private,schema_id:SCHEMA.into(),schema_version:1,readers:vec![],writers:vec![]},
                StateNamespaceDeclaration {name:format!("{ID}.shared"),visibility:StateNamespaceVisibility::Shared,schema_id:SCHEMA.into(),schema_version:1,readers:vec![],writers:vec![]},
            ]}
    }
    fn initialize(profile:HostProfile)->Result<(),CompatibilityError> {
        if profile.abi.major==ABI_VERSION.major && profile.abi.minor==ABI_VERSION.minor {Ok(())}
        else {Err(CompatibilityError {kind:CompatibilityErrorKind::AbiVersionMismatch,subject:ID.into(),message:"test ABI mismatch".into()})}
    }
}
impl hooks::Guest for SemanticStateAddon {
    fn invoke(input:HookInvocation)->Result<HookOutput,AddonError> {
        let HookPayload::Structure(p)=input.payload else {return Err(error("Structure required"));};
        if p.candidate.raw_node_id!=0 {
            return Ok(HookOutput {decision:HookDecision::ContinueProcessing,replacement:None,effects:effects()});
        }
        let mode=state_store::get(StateScope::Project,StateNamespaceVisibility::Private,NS,"mode")
            .map_err(|e|error(e.message))?.map(|v|v.bytes).unwrap_or_else(||b"local".to_vec());
        let (scope,visibility,namespace)=if mode==b"shared" {
            (StateScope::Parse,StateNamespaceVisibility::Shared,format!("{ID}.shared"))
        } else if mode==b"document" {(StateScope::Document,StateNamespaceVisibility::Private,NS.into())}
        else {(StateScope::Parse,StateNamespaceVisibility::Private,NS.into())};
        state_store::put(scope,visibility,&namespace,"value",&StateValue {schema_id:SCHEMA.into(),encoding:StateEncoding::Raw,bytes:b"accepted".to_vec()})
            .map_err(|e|error(e.message))?;
        if mode==b"caught-error" {let _=state_store::get(StateScope::Parse,StateNamespaceVisibility::Private,"not-declared","missing");}
        Ok(HookOutput {decision:HookDecision::ContinueProcessing,replacement:None,effects:effects()})
    }
}
impl text_macro::Guest for SemanticStateAddon {fn expand(_:TextMacroInput)->Result<TextMacroOutput,AddonError>{Err(error("no macro"))}}
impl tree_macro::Guest for SemanticStateAddon {fn expand(_:TreeMacroInput)->Result<TreeMacroOutput,AddonError>{Err(error("no macro"))}}
impl ast_macro::Guest for SemanticStateAddon {fn expand(_:AstMacroInput)->Result<AstMacroOutput,AddonError>{Err(error("no macro"))}}
