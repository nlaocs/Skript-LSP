use super::{
    append_metadata, continue_with_mode, register_handler, reject_structure, structure_warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    ContextUpdate, HookOutput, InvocationContext, RegisteredSyntaxHandler, StructureBodyMode,
    StructurePayload, StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructExample";
const HANDLER_ID: &str = "core.structure.struct-example";
const FUNCTION_EVENT: &str = "ch.njol.skript.lang.function.FunctionEvent";
const INTRODUCED_IN: (u64, u64) = (2, 10);
const FIRST_UNSUPPORTED_MINOR: (u64, u64) = (2, 17);

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    payload.candidate.handler.as_deref() == Some(HANDLER_ID)
        || crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: StructurePayload) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    if entering {
        match version_support(INTRODUCED_IN) {
            VersionSupport::TooOld => {
                return reject_structure("StructExample is not available before Skript 2.10");
            }
            VersionSupport::Unresolved => {
                return unresolved_structure(
                    payload,
                    "core.struct-example.unresolved-version",
                    "Skript version is missing or newer than the supported 2.16 range; StructExample semantics are unresolved",
                );
            }
            VersionSupport::Supported => {}
        }
    }
    let feature_enabled = crate::experiments::enabled(&payload.context, "examples");
    if entering && !feature_enabled {
        return reject_structure("the `examples` experiment is not enabled");
    }
    if entering {
        append_metadata(&mut payload, "experimental-feature", "examples");
        append_metadata(&mut payload, "experimental-feature-state", "enabled");
    }
    let mut output = continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Trigger,
        "example-structure",
        "core.structure.example",
    );
    if entering {
        // StructExample loads its body exactly like a Function body so parse
        // errors are retained, then discards the resulting Trigger.
        output.effects.context_updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.event-classes".to_owned(),
            value: Some(FUNCTION_EVENT.as_bytes().to_vec()),
        });
        output.effects.context_updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.event-name".to_owned(),
            value: Some(b"example".to_vec()),
        });
        output.effects.context_updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.delay-state".to_owned(),
            value: Some(b"false".to_vec()),
        });
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionSupport {
    Supported,
    TooOld,
    Unresolved,
}

fn version_support(introduced_in: (u64, u64)) -> VersionSupport {
    let version = crate::runtime::current().and_then(|profile| {
        profile
            .skript_version
            .and_then(|version| crate::runtime::parse_skript_version(&version))
    });
    version_support_for(version, introduced_in)
}

fn version_support_for(version: Option<(u64, u64)>, introduced_in: (u64, u64)) -> VersionSupport {
    match version {
        Some(version) if version < introduced_in => VersionSupport::TooOld,
        Some(version) if version >= FIRST_UNSUPPORTED_MINOR => VersionSupport::Unresolved,
        Some(_) => VersionSupport::Supported,
        None => VersionSupport::Unresolved,
    }
}

fn unresolved_structure(payload: StructurePayload, code: &str, message: &str) -> HookOutput {
    let span = payload.candidate.span.clone();
    super::continue_unresolved(payload, vec![structure_warning(code, message, span)])
}

#[cfg(test)]
mod tests {
    use super::{FIRST_UNSUPPORTED_MINOR, VersionSupport, version_support_for};

    #[test]
    fn example_uses_the_known_2_10_boundary() {
        assert_eq!(
            version_support_for(Some((2, 9)), (2, 10)),
            VersionSupport::TooOld
        );
        assert_eq!(
            version_support_for(Some((2, 10)), (2, 10)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some((2, 16)), (2, 10)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some(FIRST_UNSUPPORTED_MINOR), (2, 10)),
            VersionSupport::Unresolved
        );
        assert_eq!(
            version_support_for(None, (2, 10)),
            VersionSupport::Unresolved
        );
    }
}
