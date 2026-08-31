use super::{append_metadata, context_value_update, continue_with_mode, structure_warning};
use crate::experiments::ExperimentPhase;
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, RegisteredSyntaxHandler, StructureBodyMode, StructurePayload,
    StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructUsing";
const HANDLER_ID: &str = "core.structure.struct-using";
const INTRODUCED_IN: (u64, u64) = (2, 9);
const FIRST_UNSUPPORTED_MINOR: (u64, u64) = (2, 17);

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    payload.candidate.handler.as_deref() == Some(HANDLER_ID)
        || crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: StructurePayload) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    if !entering {
        return continue_with_mode(
            &context,
            payload,
            StructureBodyMode::None,
            "using-experiment",
            "core.structure.using",
        );
    }
    match version_support(INTRODUCED_IN) {
        VersionSupport::TooOld => {
            return super::reject_structure("StructUsing is not available before Skript 2.9");
        }
        VersionSupport::Unresolved => {
            return unresolved_structure(
                payload,
                "core.struct-using.unresolved-version",
                "Skript version is missing or newer than the supported 2.16 range; StructUsing semantics are unresolved",
            );
        }
        VersionSupport::Supported => {}
    }
    let Some(raw_name) = payload.candidate.regex_captures.first().cloned() else {
        return unresolved_structure(
            payload,
            "core.struct-using.unresolved-name",
            "the experiment name was not exposed by the Structure capture",
        );
    };
    let name = raw_name.trim().to_owned();
    let experiment = match crate::experiments::find(&name) {
        Ok(experiment) => experiment,
        Err(reason) => {
            return unresolved_structure(
                payload,
                "core.struct-using.unresolved-registry",
                format!("the experiment registry could not be read: {reason}"),
            );
        }
    };
    append_metadata(&mut payload, "experiment-name", &name);
    append_metadata(
        &mut payload,
        "experiment-state",
        if experiment.is_some() {
            "resolved"
        } else {
            "unknown"
        },
    );
    if let Some(experiment) = &experiment {
        append_metadata(&mut payload, "experiment-code-name", &experiment.code_name);
    }
    let mut output = continue_with_mode(
        &context,
        payload,
        StructureBodyMode::None,
        "using-experiment",
        "core.structure.using",
    );
    match experiment {
        Some(experiment) => {
            output.effects.context_updates.push(context_value_update(
                &context,
                &crate::experiments::context_key(&experiment.code_name),
                "true",
            ));
            if let Some((code, message)) = phase_warning(experiment.phase, &experiment.code_name) {
                output.effects.diagnostics.push(structure_warning(
                    code,
                    message,
                    output_span(&output),
                ));
            }
        }
        None => output.effects.diagnostics.push(structure_warning(
            "core.struct-using.unknown-experiment",
            format!("the experimental feature `{name}` was not found"),
            output_span(&output),
        )),
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

fn unresolved_structure(
    payload: StructurePayload,
    code: impl Into<String>,
    message: impl Into<String>,
) -> HookOutput {
    let span = payload.candidate.span.clone();
    super::continue_unresolved(payload, vec![structure_warning(code, message, span)])
}

fn phase_warning(phase: ExperimentPhase, code_name: &str) -> Option<(&'static str, String)> {
    match phase {
        ExperimentPhase::Deprecated => Some((
            "core.struct-using.deprecated-experiment",
            format!(
                "the experimental feature `{code_name}` is deprecated and may be removed in future versions."
            ),
        )),
        ExperimentPhase::Mainstream => Some((
            "core.struct-using.mainstream-experiment",
            format!(
                "the experimental feature `{code_name}` is now included by default and is no longer required."
            ),
        )),
        ExperimentPhase::Stable | ExperimentPhase::Experimental => None,
    }
}

fn output_span(output: &HookOutput) -> crate::nlaocs::skript_parser_addon::types::MappedSpan {
    match output.replacement.as_ref() {
        Some(crate::nlaocs::skript_parser_addon::types::HookPayload::Structure(payload)) => {
            payload.candidate.span.clone()
        }
        _ => unreachable!("Structure continuation always retains its payload"),
    }
}

#[cfg(test)]
mod tests {
    use super::{FIRST_UNSUPPORTED_MINOR, VersionSupport, phase_warning, version_support_for};
    use crate::experiments::ExperimentPhase;

    #[test]
    fn warns_for_deprecated_experiments() {
        let (code, message) = phase_warning(ExperimentPhase::Deprecated, "legacy feature")
            .expect("deprecated experiments must produce a warning");
        assert_eq!(code, "core.struct-using.deprecated-experiment");
        assert!(message.contains("legacy feature"));
        assert!(message.contains("deprecated"));
    }

    #[test]
    fn does_not_warn_for_active_experimental_features() {
        assert!(phase_warning(ExperimentPhase::Experimental, "preview").is_none());
        assert!(phase_warning(ExperimentPhase::Stable, "stable").is_none());
    }

    #[test]
    fn recognizes_the_struct_using_version_boundary() {
        assert_eq!(
            version_support_for(Some((2, 8)), (2, 9)),
            VersionSupport::TooOld
        );
        assert_eq!(
            version_support_for(Some((2, 9)), (2, 9)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some((2, 16)), (2, 9)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some(FIRST_UNSUPPORTED_MINOR), (2, 9)),
            VersionSupport::Unresolved
        );
        assert_eq!(
            version_support_for(None, (2, 9)),
            VersionSupport::Unresolved
        );
    }
}
