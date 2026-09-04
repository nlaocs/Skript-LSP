use super::{
    append_metadata, context_value, continue_with_mode, reject_structure, structure_error,
    structure_warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, HookOutput, InvocationContext, ParseSummary, RegisteredSyntaxHandler,
    StructureBodyMode, StructureNodeType, StructurePayload, StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructAutoReload";
const HANDLER_ID: &str = "core.structure.struct-auto-reload";
const STRING_TYPE: &str = "java.lang.String";
const INTRODUCED_IN: (u64, u64) = (2, 13);
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
    if entering {
        let span = payload.candidate.span.clone();
        match version_support(INTRODUCED_IN) {
            VersionSupport::TooOld => {
                return super::reject_structure(
                    "StructAutoReload is not available before Skript 2.13",
                );
            }
            VersionSupport::Unresolved => {
                return unresolved_structure(
                    payload,
                    vec![structure_warning(
                        "core.struct-auto-reload.unresolved-version",
                        "Skript version is missing or newer than the supported 2.16 range; StructAutoReload semantics are unresolved",
                        span,
                    )],
                );
            }
            VersionSupport::Supported => {}
        }
    }
    let body_mode = match payload.candidate.actual_node_type {
        StructureNodeType::Simple => StructureBodyMode::None,
        StructureNodeType::Section | StructureNodeType::Both => StructureBodyMode::Entries,
    };
    let mut diagnostics = Vec::new();
    if entering {
        append_metadata(&mut payload, "auto-reload.runtime-check", "unresolved");
        let mut runtime_unresolved = false;
        match context_value(&payload, "core.script-loader.async") {
            Some("false") => {
                return reject_structure("auto reload requires asynchronous script loading");
            }
            Some("true") => {}
            _ => {
                runtime_unresolved = true;
                diagnostics.push(structure_warning(
                    "core.struct-auto-reload.unresolved-loader-mode",
                    "the host did not expose whether ScriptLoader is asynchronous; AutoReload runtime validation is unresolved",
                    payload.candidate.span.clone(),
                ));
            }
        }
        match context_value(&payload, "core.script.file.exists") {
            Some("false") => return reject_structure("auto reload requires a real script file"),
            Some("true") => {}
            _ => {
                runtime_unresolved = true;
                diagnostics.push(structure_warning(
                    "core.struct-auto-reload.unresolved-script-file",
                    "the host did not expose whether the current script file exists; AutoReload runtime validation is unresolved",
                    payload.candidate.span.clone(),
                ));
            }
        }
        if runtime_unresolved {
            // The node shape is still known even when runtime validation is
            // unavailable; retain entries for diagnostics without publishing
            // the AutoReload semantic context.
            if entering {
                payload.candidate.body_mode = body_mode;
            }
            return unresolved_structure(payload, diagnostics);
        }
    } else {
        diagnostics.extend(validate_entries(&payload));
    }
    let mut output = continue_with_mode(
        &context,
        payload,
        body_mode,
        "auto-reload",
        "core.structure.auto-reload",
    );
    output.effects.diagnostics.extend(diagnostics);
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

fn unresolved_structure(payload: StructurePayload, diagnostics: Vec<Diagnostic>) -> HookOutput {
    super::continue_unresolved(payload, diagnostics)
}

fn validate_entries(
    payload: &StructurePayload,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let mut diagnostics = Vec::new();
    for entry in &payload.candidate.entries {
        if entry.key.eq_ignore_ascii_case("permission") && entry.source.trim().is_empty() {
            diagnostics.push(structure_error(
                "core.struct-auto-reload.empty-permission",
                "AutoReload permission cannot be empty",
                entry.span.clone(),
            ));
        }
        if entry.key.eq_ignore_ascii_case("recipients")
            && recipients_summary_is_unresolved(entry.value_summary.as_ref())
        {
            diagnostics.push(structure_warning(
                "core.struct-auto-reload.unresolved-recipients",
                "the AutoReload recipients expression has no resolved String value summary",
                entry.span.clone(),
            ));
        }
    }
    diagnostics
}

fn recipients_summary_is_unresolved(summary: Option<&ParseSummary>) -> bool {
    let Some(summary) = summary else {
        return true;
    };
    if !summary.kind.eq_ignore_ascii_case("expression") {
        return true;
    }
    summary.return_type.as_deref() != Some(STRING_TYPE)
        && !summary
            .possible_return_types
            .iter()
            .any(|return_type| return_type == STRING_TYPE)
}

#[cfg(test)]
mod tests {
    use super::{
        FIRST_UNSUPPORTED_MINOR, STRING_TYPE, VersionSupport, recipients_summary_is_unresolved,
        version_support_for,
    };
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, ParseSummary,
    };

    fn summary(return_type: Option<&str>, possible_return_types: &[&str]) -> ParseSummary {
        ParseSummary {
            kind: "expression".to_owned(),
            definition_id: None,
            registration_id: None,
            element_class: None,
            pattern_index: None,
            return_type: return_type.map(str::to_owned),
            possible_return_types: possible_return_types
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Multiple),
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn recipients_use_a_resolved_string_value_summary() {
        assert!(!recipients_summary_is_unresolved(Some(&summary(
            Some(STRING_TYPE),
            &[],
        ))));
        assert!(!recipients_summary_is_unresolved(Some(&summary(
            None,
            &[STRING_TYPE],
        ))));
        assert!(recipients_summary_is_unresolved(None));
    }

    #[test]
    fn auto_reload_requires_a_known_supported_version() {
        assert_eq!(
            version_support_for(Some((2, 13)), (2, 13)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some((2, 16)), (2, 13)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some((2, 12)), (2, 13)),
            VersionSupport::TooOld
        );
        assert_eq!(
            version_support_for(Some(FIRST_UNSUPPORTED_MINOR), (2, 13)),
            VersionSupport::Unresolved
        );
        assert_eq!(
            version_support_for(None, (2, 13)),
            VersionSupport::Unresolved
        );
    }
}
