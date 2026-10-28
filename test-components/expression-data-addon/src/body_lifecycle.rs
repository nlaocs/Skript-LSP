//! An isolated fixture variant: body hooks are inert; fresh Exit can fail.
use crate::exports::nlaocs::skript_parser_addon::{addon,ast_macro,hooks,text_macro,tree_macro};
use crate::nlaocs::skript_parser_addon::{state_store,types::*};
use parser_wasm::{ABI_VERSION,CAPABILITY_HOOKS,CAPABILITY_STRUCTURE_PARSER,
    CAPABILITY_EXPRESSION_PARSER,CAPABILITY_STATE_STORE,CAPABILITY_INCREMENTAL_STRUCTURE_BODY};

const ID:&str="test.body-lifecycle";
const INERT:&str="body-lifecycle.inert";
const EXIT:&str="body-lifecycle.exit";
const NAMESPACE:&str="body-lifecycle";
const SCHEMA:&str="nlaocs.test.body-lifecycle";
pub(crate) struct BodyLifecycleAddon;

fn empty()->HookEffects {HookEffects {diagnostics:vec![],context_updates:vec![],parse_requests:vec![],parse_results:vec![]}}
fn error(message:impl Into<String>)->AddonError {AddonError {kind:AddonErrorKind::InvalidPayload,message:message.into(),diagnostics:vec![]}}
fn selector()->HookSelector {HookSelector {pattern_index:None,pattern_source:None,mark:None,tags:vec![],captures:vec![],return_type:None,multiplicity:None,metadata:vec![]}}

impl addon::Guest for BodyLifecycleAddon {
    fn prepare_source_step(context: crate::nlaocs::skript_parser_addon::types::SourcePreparationContext)
        -> Result<crate::nlaocs::skript_parser_addon::types::SourcePreparationStatus, crate::nlaocs::skript_parser_addon::types::AddonError> {
        let _ = context; Ok(crate::nlaocs::skript_parser_addon::types::SourcePreparationStatus::Ready("unused".into()))
    }

    fn manifest()->ComponentManifest {
        ComponentManifest {
            component_id:ID.into(),component_version:env!("CARGO_PKG_VERSION").into(),
            abi:AbiVersion {major:ABI_VERSION.major,minor:ABI_VERSION.minor},
            capabilities:[CAPABILITY_HOOKS,CAPABILITY_EXPRESSION_PARSER,CAPABILITY_STRUCTURE_PARSER,
                CAPABILITY_STATE_STORE,CAPABILITY_INCREMENTAL_STRUCTURE_BODY].into_iter()
                .map(|id|CapabilityRequirement {id:id.into(),minimum_version:1,required:true}).collect(),
            incremental_local_subscriptions:vec![],incremental_header_handlers:vec![],
            incremental_body_handler_subscriptions:vec![],
            // The complete Expression hook has no reads, writes or hidden state.
            // Stateful Structure Enter/Exit is deliberately NOT opted in.
            incremental_body_subscriptions:vec![INERT.into()],
            subscriptions:vec![
                HookSubscription {id:INERT.into(),target:HookTarget::SyntaxKind(SyntaxKind::Expression),phase:HookPhase::Expression,
                    priority:100,mode:HookMode::Transform,capability_id:CAPABILITY_EXPRESSION_PARSER.into(),selector:selector()},
                HookSubscription {id:EXIT.into(),target:HookTarget::SyntaxKind(SyntaxKind::Structure),phase:HookPhase::Structure,
                    priority:100,mode:HookMode::Transform,capability_id:CAPABILITY_STRUCTURE_PARSER.into(),selector:selector()},
            ],registered_syntax_handlers:vec![],catalog_annotations:vec![],
            state_namespaces:vec![StateNamespaceDeclaration {name:NAMESPACE.into(),visibility:StateNamespaceVisibility::Private,
                schema_id:SCHEMA.into(),schema_version:1,readers:vec![],writers:vec![]}],
        }
    }
    fn initialize(profile:HostProfile)->Result<(),CompatibilityError> {
        if profile.abi.major!=ABI_VERSION.major || profile.abi.minor!=ABI_VERSION.minor {
            return Err(CompatibilityError {kind:CompatibilityErrorKind::AbiVersionMismatch,subject:ID.into(),message:"fixture ABI mismatch".into()});
        }
        Ok(())
    }
}

impl hooks::Guest for BodyLifecycleAddon {
    fn invoke(input:HookInvocation)->Result<HookOutput,AddonError> {
        if input.context.subscription_id==INERT {
            return Ok(HookOutput {decision:HookDecision::ContinueProcessing,replacement:None,effects:empty()});
        }
        if input.context.subscription_id!=EXIT {return Err(error("unknown fixture subscription"));}
        let HookPayload::Structure(payload)=input.payload else {return Err(error("fixture expects Structure payload"));};
        if payload.timing==StructureTiming::EnterBody {
            return Ok(HookOutput {decision:HookDecision::ContinueProcessing,replacement:None,effects:empty()});
        }
        let mode=state_store::get(StateScope::Project,StateNamespaceVisibility::Private,NAMESPACE,"mode")
            .map_err(|e|error(e.message))?.map(|v|v.bytes).unwrap_or_else(||b"accept".to_vec());
        // A failed/rejected invocation must roll this back. Successful Exit
        // writes remain current, outside the cached body certificate.
        state_store::put(StateScope::Parse,StateNamespaceVisibility::Private,NAMESPACE,
            &format!("exit:{}",payload.candidate.raw_node_id),
            &StateValue {schema_id:SCHEMA.into(),encoding:StateEncoding::Raw,bytes:b"current-exit".to_vec()})
            .map_err(|e|error(e.message))?;
        if mode==b"fail" {return Err(error("controlled Exit failure after body"));}
        let decision=if mode==b"reject" {HookDecision::Reject(Rejection {
            reason:"controlled Exit rejection after body".into(),diagnostics:vec![],
        })} else {HookDecision::ContinueProcessing};
        Ok(HookOutput {decision,replacement:None,effects:empty()})
    }
}
impl text_macro::Guest for BodyLifecycleAddon {
    fn expand(_:TextMacroInput)->Result<TextMacroOutput,AddonError> {Err(error("fixture has no Text macro"))}
}
impl tree_macro::Guest for BodyLifecycleAddon {
    fn expand(_:TreeMacroInput)->Result<TreeMacroOutput,AddonError> {Err(error("fixture has no Tree macro"))}
}
impl ast_macro::Guest for BodyLifecycleAddon {
    fn expand(_:AstMacroInput)->Result<AstMacroOutput,AddonError> {Err(error("fixture has no AST macro"))}
}
