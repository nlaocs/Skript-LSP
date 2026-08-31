use super::{
    append_metadata, continue_with_mode, direct_body_nodes, is_trivia, structure_error,
    structure_warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, RawNodeKind, RawTreeNode, RegisteredSyntaxHandler,
    StructureBodyMode, StructurePayload, StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructOptions";
const HANDLER_ID: &str = "core.structure.struct-options";
const INTRODUCED_IN: (u64, u64) = (2, 7);
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
        match version_support(INTRODUCED_IN) {
            VersionSupport::TooOld => {
                return super::reject_structure(
                    "StructOptions is not available through the modern Structure API before Skript 2.7",
                );
            }
            VersionSupport::Unresolved => {
                return unresolved_structure(
                    payload,
                    "core.struct-options.unresolved-version",
                    "Skript version is missing or newer than the supported 2.16 range; StructOptions semantics are unresolved",
                );
            }
            VersionSupport::Supported => {}
        }
    }
    let diagnostics = if entering {
        validate_body(&payload)
    } else {
        Vec::new()
    };
    if entering {
        append_metadata(&mut payload, "options-scope", "script");
    }
    let mut output = continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Raw,
        "script-options",
        "core.structure.options",
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

fn unresolved_structure(payload: StructurePayload, code: &str, message: &str) -> HookOutput {
    let span = payload.candidate.span.clone();
    super::continue_unresolved(payload, vec![structure_warning(code, message, span)])
}

fn validate_body(
    payload: &StructurePayload,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    direct_body_nodes(payload)
        .into_iter()
        .filter(|node| !is_trivia(node))
        .flat_map(|node| validate_node(payload, node))
        .collect()
}

fn validate_node(
    payload: &StructurePayload,
    node: &RawTreeNode,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    match node.kind {
        RawNodeKind::Simple => validate_option_line(node),
        RawNodeKind::Section => {
            let mut diagnostics = if node.text.trim().is_empty() {
                vec![structure_error(
                    "core.struct-options.empty-section",
                    "an options section must have a non-empty option name",
                    node.span.clone(),
                )]
            } else {
                Vec::new()
            };
            for child_id in &node.children {
                if let Some(child) = payload
                    .body_tree
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id == *child_id)
                    && !is_trivia(child)
                {
                    diagnostics.extend(validate_node(payload, child));
                }
            }
            diagnostics
        }
        RawNodeKind::Invalid => vec![structure_error(
            "core.struct-options.invalid-entry",
            "this options entry is not a valid Skript source line",
            node.span.clone(),
        )],
        RawNodeKind::Blank | RawNodeKind::Comment => Vec::new(),
    }
}

fn validate_option_line(
    node: &RawTreeNode,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let separator = node.text.find(':');
    let Some(separator) = separator else {
        return vec![structure_error(
            "core.struct-options.missing-separator",
            "an options entry must contain `:` between its name and value",
            node.span.clone(),
        )];
    };
    if node.text[..separator].trim().is_empty() {
        return vec![structure_error(
            "core.struct-options.empty-name",
            "an options entry must have a non-empty name",
            node.span.clone(),
        )];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{FIRST_UNSUPPORTED_MINOR, VersionSupport, version_support_for};

    #[test]
    fn options_use_the_modern_structure_boundary() {
        assert_eq!(
            version_support_for(Some((2, 6)), (2, 7)),
            VersionSupport::TooOld
        );
        assert_eq!(
            version_support_for(Some((2, 7)), (2, 7)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some((2, 16)), (2, 7)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some(FIRST_UNSUPPORTED_MINOR), (2, 7)),
            VersionSupport::Unresolved
        );
        assert_eq!(
            version_support_for(None, (2, 7)),
            VersionSupport::Unresolved
        );
    }
}
