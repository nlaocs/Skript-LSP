use super::{
    append_metadata, continue_with_mode, direct_body_nodes, is_trivia, mapped_subspan,
    parse_context_options, request_parses, structure_error, structure_warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    ExpressionExpectedType, ExpressionTypeOption, HookOutput, InvocationContext, MappedSpan,
    MetadataEntry, ParseRequest, ParseResult, ParseResultStatus, RawNodeKind,
    RegisteredSyntaxHandler, StructureBodyMode, StructurePayload, StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructVariables";
const HANDLER_ID: &str = "core.structure.struct-variables";
const INTRODUCED_IN: (u64, u64) = (2, 7);
const FIRST_UNSUPPORTED_MINOR: (u64, u64) = (2, 17);

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    payload.candidate.handler.as_deref() == Some(HANDLER_ID)
        || crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(
    context: InvocationContext,
    mut payload: StructurePayload,
    parse_results: &[ParseResult],
) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    if entering {
        match version_support(INTRODUCED_IN) {
            VersionSupport::TooOld => {
                return super::reject_structure(
                    "StructVariables is not available through the modern Structure API before Skript 2.7",
                );
            }
            VersionSupport::Unresolved => {
                return unresolved_structure(
                    payload,
                    "core.struct-variables.unresolved-version",
                    "Skript version is missing or newer than the supported 2.16 range; StructVariables semantics are unresolved",
                );
            }
            VersionSupport::Supported => {}
        }
    }
    let mut diagnostics = if entering {
        validate_body(&payload)
    } else {
        Vec::new()
    };
    if entering {
        let defaults = variable_defaults(&payload);
        let requests = default_parse_requests(&payload, &defaults);
        let pending = super::pending_parse_requests(&requests, parse_results);
        if !pending.is_empty() {
            return request_parses(payload, pending);
        }
        let valid_values = append_value_diagnostics(&requests, parse_results, &mut diagnostics);
        append_metadata(&mut payload, "variables-scope", "script");
        append_metadata(
            &mut payload,
            "variables-values",
            if valid_values { "resolved" } else { "partial" },
        );
        // Skript skips invalid defaults individually; one bad value does not
        // invalidate the Variables Structure or the other defaults.
    }
    let mut output = continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Raw,
        "script-variables",
        "core.structure.variables",
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
    unresolved_with_diagnostics(payload, vec![super::structure_warning(code, message, span)])
}

fn unresolved_with_diagnostics(
    payload: StructurePayload,
    diagnostics: Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic>,
) -> HookOutput {
    super::continue_unresolved(payload, diagnostics)
}

#[derive(Debug, Clone)]
struct VariableDefault {
    source: String,
    span: MappedSpan,
}

fn variable_defaults(payload: &StructurePayload) -> Vec<VariableDefault> {
    direct_body_nodes(payload)
        .into_iter()
        .filter(|node| matches!(node.kind, RawNodeKind::Simple))
        .filter_map(|node| {
            let separator = node.text.find('=')?;
            let raw = &node.text[separator + 1..];
            let leading = raw.len() - raw.trim_start().len();
            let source = raw.trim();
            if source.is_empty() {
                return None;
            }
            let relative_start = separator + 1 + leading;
            let start = node
                .span
                .virtual_range
                .start
                .saturating_add(relative_start as u64);
            Some(VariableDefault {
                source: source.to_owned(),
                span: mapped_subspan(&node.span, start, start.saturating_add(source.len() as u64)),
            })
        })
        .collect()
}

fn default_parse_requests(
    payload: &StructurePayload,
    defaults: &[VariableDefault],
) -> Vec<ParseRequest> {
    defaults
        .iter()
        .enumerate()
        .map(|(request_id, default)| {
            let mut options = parse_context_options(&payload.context);
            options.extend([
                MetadataEntry {
                    key: "parse.mode".to_owned(),
                    value: "literals-only".to_owned(),
                    owner_component_id: None,
                },
                MetadataEntry {
                    key: "context.value.parser.parse-context".to_owned(),
                    value: "SCRIPT".to_owned(),
                    owner_component_id: None,
                },
            ]);
            ParseRequest {
                request_id: request_id as u64,
                parser_id: "host.expression".to_owned(),
                input: default.source.clone(),
                expected_types: vec![ExpressionExpectedType {
                    class_name: "java.lang.Object".to_owned(),
                    plural: false,
                }],
                span: default.span.clone(),
                options,
            }
        })
        .collect()
}

fn append_value_diagnostics(
    requests: &[ParseRequest],
    results: &[ParseResult],
    diagnostics: &mut Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic>,
) -> bool {
    let mut valid = requests.len() == results.len();
    for request in requests {
        let result = results.iter().find(|result| {
            result.request_id == request.request_id && result.parser_id == request.parser_id
        });
        let Some(result) = result else {
            valid = false;
            diagnostics.push(structure_error(
                "core.struct-variables.missing-value-result",
                format!(
                    "the parser did not return a result for default variable value `{}`",
                    request.input
                ),
                request.span.clone(),
            ));
            continue;
        };
        if result.status != ParseResultStatus::Success {
            valid = false;
            diagnostics.push(structure_error(
                "core.struct-variables.invalid-value",
                format!(
                    "cannot understand the default variable value `{}`",
                    request.input
                ),
                request.span.clone(),
            ));
            continue;
        }
        valid &= append_serialization_diagnostics(result, request, diagnostics);
    }
    valid
}

fn append_serialization_diagnostics(
    result: &ParseResult,
    request: &ParseRequest,
    diagnostics: &mut Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic>,
) -> bool {
    let return_type = result
        .roots
        .first()
        .and_then(|root| result.nodes.iter().find(|node| node.node_id == *root))
        .and_then(|node| node.summary.as_ref())
        .and_then(|summary| summary.return_type.as_deref());
    let Some(return_type) = return_type else {
        diagnostics.push(structure_warning(
            "core.struct-variables.unresolved-value-type",
            "the default value parsed, but its return type is unavailable for persistence checks",
            request.span.clone(),
        ));
        return false;
    };
    let contract = match crate::catalog::type_serialization_contract(return_type) {
        Ok(Some(contract)) => contract,
        Ok(None) => {
            diagnostics.push(structure_warning(
                "core.struct-variables.unresolved-serializer",
                format!("no registered ClassInfo was found for `{return_type}`"),
                request.span.clone(),
            ));
            return false;
        }
        Err(reason) => {
            diagnostics.push(structure_warning(
                "core.struct-variables.unresolved-serializer",
                format!("the serializer contract could not be read: {reason}"),
                request.span.clone(),
            ));
            return false;
        }
    };
    if !contract.has_serializer {
        diagnostics.push(structure_error(
            "core.struct-variables.value-not-serializable",
            format!(
                "values of type `{}` cannot be saved in a variable",
                contract.type_class
            ),
            request.span.clone(),
        ));
        return false;
    }
    let Some(serialize_as) = contract.serialize_as else {
        return true;
    };
    match crate::catalog::can_convert(return_type, &serialize_as) {
        Ok(crate::catalog::TypeRelation::Compatible) => true,
        Ok(crate::catalog::TypeRelation::Incompatible) => {
            diagnostics.push(structure_error(
                "core.struct-variables.serialize-as-incompatible",
                format!(
                    "`{return_type}` cannot be converted to its serialization type `{serialize_as}`"
                ),
                request.span.clone(),
            ));
            false
        }
        Ok(crate::catalog::TypeRelation::Unknown) | Err(_) => {
            diagnostics.push(structure_warning(
                "core.struct-variables.unresolved-serialize-as",
                format!(
                    "conversion from `{return_type}` to serialization type `{serialize_as}` could not be proven"
                ),
                request.span.clone(),
            ));
            false
        }
    }
}

fn validate_body(
    payload: &StructurePayload,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    direct_body_nodes(payload)
        .into_iter()
        .filter(|node| !is_trivia(node))
        .flat_map(|node| match node.kind {
            RawNodeKind::Simple => validate_variable_line(node, &payload.type_options),
            RawNodeKind::Section => vec![structure_error(
                "core.struct-variables.nested-section",
                "a variables structure entry must be a simple `name = value` line",
                node.span.clone(),
            )],
            RawNodeKind::Invalid => vec![structure_error(
                "core.struct-variables.invalid-entry",
                "this variables entry is not a valid Skript source line",
                node.span.clone(),
            )],
            RawNodeKind::Blank | RawNodeKind::Comment => Vec::new(),
        })
        .collect()
}

fn validate_variable_line(
    node: &crate::nlaocs::skript_parser_addon::types::RawTreeNode,
    type_options: &[ExpressionTypeOption],
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let Some((name, value)) = node.text.split_once('=') else {
        return vec![structure_error(
            "core.struct-variables.missing-separator",
            "a variables entry must contain `=` between its name and value",
            node.span.clone(),
        )];
    };
    let mut diagnostics = validate_variable_name(name.trim(), type_options, node.span.clone());
    if value.trim().is_empty() {
        diagnostics.push(structure_error(
            "core.struct-variables.empty-value",
            "a default variable value cannot be empty",
            node.span.clone(),
        ));
    }
    diagnostics
}

fn validate_variable_name(
    name: &str,
    type_options: &[ExpressionTypeOption],
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let mut diagnostics = Vec::new();
    let inner = if let Some(inner) = name
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    {
        inner
    } else if name.contains('{') || name.contains('}') {
        diagnostics.push(structure_error(
            "core.struct-variables.invalid-name",
            "variable names must have a matching pair of braces",
            span.clone(),
        ));
        return diagnostics;
    } else {
        diagnostics.push(structure_warning(
            "core.struct-variables.unbraced-name",
            "unbraced default variable names are accepted for compatibility but are deprecated by Skript",
            span.clone(),
        ));
        name
    };
    if inner.trim().is_empty() || inner.contains('<') || inner.contains('>') {
        diagnostics.push(structure_error(
            "core.struct-variables.invalid-name",
            "a variable name must not be empty or contain angle brackets",
            span.clone(),
        ));
    }
    if inner.trim_start().starts_with('_') {
        diagnostics.push(structure_error(
            "core.struct-variables.local-name",
            "script variables cannot use a local variable name beginning with `_`",
            span.clone(),
        ));
    }
    let mut remaining = inner;
    while let Some(start) = remaining.find('%') {
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('%') else {
            diagnostics.push(structure_error(
                "core.struct-variables.unclosed-placeholder",
                "a variable name contains an unclosed `%...%` type placeholder",
                span.clone(),
            ));
            break;
        };
        let placeholder = after_start[..end].trim();
        if placeholder.is_empty() {
            diagnostics.push(structure_error(
                "core.struct-variables.empty-placeholder",
                "a variable name contains an empty `%...%` placeholder",
                span.clone(),
            ));
        } else if placeholder.contains(['{', '}', '%']) {
            diagnostics.push(structure_error(
                "core.struct-variables.invalid-placeholder",
                "a default variable type placeholder cannot contain braces or another percent sign",
                span.clone(),
            ));
        } else if !matches_type_option(placeholder, type_options) {
            diagnostics.push(structure_error(
                "core.struct-variables.unknown-placeholder-type",
                format!("unknown type `{placeholder}` in default variable name"),
                span.clone(),
            ));
        }
        remaining = &after_start[end + 1..];
    }
    diagnostics
}

fn matches_type_option(name: &str, options: &[ExpressionTypeOption]) -> bool {
    crate::types::match_user_type_option(name, options).is_some()
}

#[cfg(test)]
mod tests {
    use super::{
        FIRST_UNSUPPORTED_MINOR, VersionSupport, validate_variable_name, version_support_for,
    };
    use crate::nlaocs::skript_parser_addon::types::{MappedSpan, TextRange};

    fn span() -> MappedSpan {
        MappedSpan {
            virtual_range: TextRange { start: 0, end: 24 },
            origins: Vec::new(),
        }
    }

    #[test]
    fn braceless_names_still_validate_type_placeholders() {
        let diagnostics = validate_variable_name("values::%unknown%", &[], span());
        let codes = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();

        assert!(codes.contains(&"core.struct-variables.unbraced-name"));
        assert!(codes.contains(&"core.struct-variables.unknown-placeholder-type"));
    }

    #[test]
    fn variables_use_the_modern_structure_boundary() {
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
