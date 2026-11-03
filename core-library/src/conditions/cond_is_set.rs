use super::{accept, annotate, matches, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.cond-is-set";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, ".CondIsSet");
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    matches(&payload, HANDLER_ID).then(|| {
        annotate(&mut payload, "semantic-mode", "existence");
        super::local_effects::existence(&mut payload);
        // CondIsSet.getAll length/XOR, restricted to one native scalar operand.
        if crate::catalog::standard_condition_source(&payload.candidate,"ch.njol.skript.conditions.CondIsSet")
            && payload.candidate.children.len()==1
            && super::child(&payload,0).is_some_and(|c|c.multiplicity==Some(crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity::Single))
            && let Some(capture)=super::child_capture_index(&payload,0)
            && payload.candidate.pattern_index<=1 {
            let negated=payload.candidate.pattern_index==1;
            annotate(&mut payload,"skript.semantic.type-guard",&serde_json::json!({"version":2,"truthDomain":"boolean","subjectCapture":capture,
                "whenTrue":{"presence":if negated {"absent"}else{"present"}},
                "whenFalse":{"presence":if negated {"present"}else{"absent"}}}).to_string());
            payload.candidate.metadata.push(crate::public_data::preserve_binding_values());
            payload.candidate.metadata.push(crate::public_data::preserve_evaluation_context());
            let input=super::child(&payload,0).and_then(|child| {
                let keys=["skript.semantic.variable-read","parser.semantic-transfer.v1","skript.semantic.evaluation-context"];
                let keys=keys.into_iter().filter(|key|child.metadata.iter().filter(|m|m.key==*key).count()==1).collect::<Vec<_>>();
                (!keys.is_empty()&&child.public_data.iter().filter(|p|p.schema_id==crate::public_data::VARIABLE_IDENTITY_SCHEMA_ID).count()==1)
                    .then(||serde_json::json!({"capture":capture,"metadataKeys":keys,"publicSchemas":[crate::public_data::VARIABLE_IDENTITY_SCHEMA_ID]}))
            });
            let keys=["skript.semantic.type-guard","skript.semantic.binding-values","skript.semantic.local-effects","skript.semantic.operand-evaluation","skript.semantic.evaluation-context"];
            let keys=keys.into_iter().filter(|key|payload.candidate.metadata.iter().filter(|m|m.key==*key).count()==1).collect::<Vec<_>>();
            annotate(&mut payload,"parser.semantic.intrinsic-facts.v1",&serde_json::json!({"version":1,"metadataKeys":keys,"inputs":input.into_iter().collect::<Vec<_>>()} ).to_string());
        }
        accept(payload)
    })
}
