use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler_with_all_type_options,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, ExpressionTypeOption,
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprParse";
const HANDLER_ID: &str = "core.expression.expr-parse";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_all_type_options(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_parse_expression(payload))
}

fn resolve_parse_expression(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    if let Some((child, target)) = payload.children.iter().find_map(|child| {
        metadata_value(&child.metadata, "target-class").map(|target| (child, target))
    }) {
        let reject = |reason: String| target_rejection(payload, child, target, reason);
        if target == "java.lang.String" {
            return reject(format!(
                "cannot parse text as {}: parsing text as text is not supported",
                child.text
            ));
        }
        match metadata_value(&child.metadata, "has-parser") {
            Some("false") => {
                return reject(format!(
                    "target type '{}' ({target}) has no parser",
                    child.text
                ));
            }
            Some("true") => {}
            _ => return unresolved("target type parser availability is unknown"),
        }
        let Some(modern_types) = crate::runtime::skript_at_least(2, 8) else {
            return unresolved(
                "Skript version is unavailable, so the required parser context is unresolved",
            );
        };
        let required_context = parse_type_context(modern_types);
        // Snapshot contexts are positive observations, not a complete negative
        // capability table: collection may omit a context after a callback error.
        if !metadata_value(&child.metadata, "type-parse-contexts").is_some_and(|contexts| {
            contexts
                .split(',')
                .any(|context| context.eq_ignore_ascii_case(required_context))
        }) {
            return unresolved(&format!(
                "target type '{}' ({target}): parser support for {required_context} is unconfirmed",
                child.text
            ));
        }
        let mut facts = vec![metadata("semantic-mode", "parse-type")];
        if let Some(classes) = closed_output_classes(payload, child, target) {
            facts.push(metadata(crate::public_data::CHANGE_INPUT_METADATA_KEY, &serde_json::json!({
                "version":2, "kind":"default-clone", "runtimeClasses":classes, "classesComplete":true,
            }).to_string()));
        }
        return resolved_with_possible_types(
            target.to_owned(),
            vec![target.to_owned()],
            ExpressionPossibleReturnTypesState::Complete,
            DynamicMultiplicity::Single,
            facts,
        );
    }
    let Some(pattern) = payload.regex_captures.first().map(|value| unquote(value)) else {
        return SemanticResolution::Reject("parse Expression has no static target".to_owned());
    };
    let Some(modern_types) = crate::runtime::skript_at_least(2, 8) else {
        return unresolved(
            "Skript version is unavailable, so ExprParse's parser context is unresolved",
        );
    };
    let required_context = parse_type_context(modern_types);
    let placeholders =
        match parse_pattern_placeholders(&pattern, &payload.type_options, required_context) {
            Ok(placeholders) => placeholders,
            Err(error) if error.capability_unknown => return unresolved(&error.reason),
            Err(error) => return SemanticResolution::Reject(error.reason),
        };
    let Some(pattern_can_be_single) = crate::runtime::skript_at_least(2, 7) else {
        return unresolved(
            "Skript version is unavailable, so ExprParse multiplicity is unresolved",
        );
    };
    let (return_type, multiplicity) =
        pattern_semantics(&placeholders, modern_types, pattern_can_be_single);
    let possible_return_types = vec![return_type.clone()];
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        vec![metadata("semantic-mode", "parse-pattern")],
    )
}

/// The standard parser's successful output classes, not a parse-success claim.
/// Addon/world-dependent parsers remain unexecuted and receive no certificate.
fn closed_output_classes(
    payload: &RegisteredExpressionPayload,
    child: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild,
    target: &str,
) -> Option<Vec<String>> {
    if !crate::catalog::standard_expression_source(payload, "ch.njol.skript.expressions.ExprParse")
    {
        return None;
    }
    let registration = core_field(&child.metadata, "type-registration-id")?;
    if core_field(&child.metadata, "target-class")? != target {
        return None;
    }
    // A typed target does not request the regex-pattern all-type payload.
    // Reuse the host lookup and bind it to the actual selected registration;
    // a different/opaque registration cannot borrow the standard parser proof.
    let active = crate::catalog::type_for_class(target).ok().flatten()?;
    if active.registration_id != registration
        || active.class_name != target
        || !crate::catalog::standard_primitive_parser(&active)
    {
        return None;
    }
    // Pinned 2.15.4 NumberParser.parse returns Integer/Double or null. The
    // virtual formatter for arbitrary Number subclasses is a separate contract.
    match active.class_name.as_str() {
        "java.lang.Number" => Some(vec!["java.lang.Integer".into(), "java.lang.Double".into()]),
        _ => None,
    }
}
fn core_field<'a>(
    entries: &'a [crate::nlaocs::skript_parser_addon::types::MetadataEntry],
    key: &str,
) -> Option<&'a str> {
    let mut found = entries.iter().filter(|entry| {
        entry.key == key
            || entry
                .key
                .rsplit_once('/')
                .is_some_and(|(_, suffix)| suffix == key)
    });
    let entry = found.next()?;
    (found.next().is_none() && entry.owner_component_id.as_deref() == Some(crate::COMPONENT_ID))
        .then_some(entry.value.as_str())
}

/// `ExprParse` used `COMMAND` until Skript 2.8 introduced the dedicated
/// `PARSE` context. Addon parsers can distinguish these contexts, so treating
/// every supported release as modern changes which target types are accepted.
fn parse_type_context(modern: bool) -> &'static str {
    if modern { "PARSE" } else { "COMMAND" }
}

fn pattern_semantics(
    placeholders: &[ParsedPlaceholder],
    modern_types: bool,
    pattern_can_be_single: bool,
) -> (String, DynamicMultiplicity) {
    let return_type = if modern_types {
        match placeholders {
            [only] => only.class_name.clone(),
            _ => "java.lang.Object".to_owned(),
        }
    } else {
        "java.lang.Object".to_owned()
    };
    // 2.6.x always exposed pattern parsing as a multiple Object result. 2.7 learned that a
    // one-placeholder pattern can be single, while 2.8 also started publishing that placeholder's
    // concrete return type and respecting whether it is plural.
    let multiplicity = if !pattern_can_be_single {
        DynamicMultiplicity::Multiple
    } else if modern_types {
        if placeholders.len() <= 1 && !placeholders.iter().any(|value| value.plural) {
            DynamicMultiplicity::Single
        } else {
            DynamicMultiplicity::Multiple
        }
    } else if placeholders.len() <= 1 {
        DynamicMultiplicity::Single
    } else {
        DynamicMultiplicity::Multiple
    };
    (return_type, multiplicity)
}

// Transport a range only when the native capture identifies this exact target.
// Unbound regex captures currently have no mapped-span transport; those errors
// retain the parent range rather than searching user text for a guessed match.
fn target_rejection(
    payload: &RegisteredExpressionPayload,
    child: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild,
    target: &str,
    reason: String,
) -> SemanticResolution {
    use crate::nlaocs::skript_parser_addon::types::ParseResultStatus;
    let mut captures = payload.parsed_captures.iter().filter(|capture| {
        matches!(capture.status, ParseResultStatus::Success)
            && capture.text == child.text
            && capture.summary.as_ref().is_some_and(|summary| {
                summary.return_type.as_deref() == child.return_type.as_deref()
                    && metadata_value(&summary.metadata, "target-class") == Some(target)
            })
            && payload.span.virtual_range.start <= capture.span.virtual_range.start
            && capture.span.virtual_range.end <= payload.span.virtual_range.end
    });
    let first = captures.next();
    match (first, captures.next()) {
        (Some(capture), None) => SemanticResolution::RejectAt {
            reason,
            span: capture.span.clone(),
        },
        _ => SemanticResolution::Reject(reason),
    }
}

#[derive(Debug)]
struct PatternError {
    reason: String,
    capability_unknown: bool,
}
impl From<String> for PatternError {
    fn from(reason: String) -> Self {
        Self {
            reason,
            capability_unknown: false,
        }
    }
}

struct ParsedPlaceholder {
    class_name: String,
    plural: bool,
}

fn parse_pattern_placeholders(
    pattern: &str,
    options: &[ExpressionTypeOption],
    required_context: &str,
) -> Result<Vec<ParsedPlaceholder>, PatternError> {
    let mut placeholders = Vec::new();
    let mut unconfirmed_context = None;
    let mut groups = Vec::<(char, bool)>::new();
    let mut cursor = 0;
    while cursor < pattern.len() {
        let character = pattern[cursor..]
            .chars()
            .next()
            .expect("cursor is on a character boundary");
        let width = character.len_utf8();
        match character {
            '\\' => {
                cursor += width;
                let Some(escaped) = pattern[cursor..].chars().next() else {
                    return Err("parse pattern ends in an unescaped backslash"
                        .to_owned()
                        .into());
                };
                cursor += escaped.len_utf8();
            }
            '<' => {
                let Some(relative_end) = pattern[cursor + width..].find('>') else {
                    return Err("parse pattern has an unclosed regex".to_owned().into());
                };
                cursor += width + relative_end + 1;
            }
            '>' => {
                return Err("parse pattern has an unexpected closing regex"
                    .to_owned()
                    .into());
            }
            '(' => {
                groups.push(('(', false));
                cursor += width;
            }
            '|' => {
                let Some(('(', has_pipe)) = groups.last_mut() else {
                    return Err("parse pattern uses '|' outside a group".to_owned().into());
                };
                *has_pipe = true;
                cursor += width;
            }
            ')' => {
                let Some(('(', has_pipe)) = groups.pop() else {
                    return Err("parse pattern has an unexpected ')'".to_owned().into());
                };
                if !has_pipe {
                    return Err("parse pattern group contains no '|' choice"
                        .to_owned()
                        .into());
                }
                cursor += width;
            }
            '[' => {
                groups.push(('[', false));
                cursor += width;
            }
            ']' => {
                if groups.pop().is_none_or(|(kind, _)| kind != '[') {
                    return Err("parse pattern has an unexpected ']'".to_owned().into());
                }
                cursor += width;
            }
            '%' => {
                let body_start = cursor + width;
                let Some(relative_end) = pattern[body_start..].find('%') else {
                    return Err("parse pattern has an unclosed type placeholder"
                        .to_owned()
                        .into());
                };
                let body_end = body_start + relative_end;
                let body = &pattern[body_start..body_end];
                let (option, plural) = crate::types::match_type_option(body, options)
                    .ok_or_else(|| format!("unknown type in parse pattern: {body}"))?;
                if !option.has_parser {
                    return Err(format!("type has no parser: {}", option.code_name).into());
                }
                if !option
                    .parse_contexts
                    .iter()
                    .any(|context| context.eq_ignore_ascii_case(required_context))
                {
                    unconfirmed_context.get_or_insert_with(|| {
                        format!(
                            "type parser support for {required_context} is unconfirmed: {}",
                            option.code_name
                        )
                    });
                }
                placeholders.push(ParsedPlaceholder {
                    class_name: option.class_name.clone(),
                    plural,
                });
                cursor = body_end + 1;
            }
            _ => cursor += width,
        }
    }
    if let Some((kind, _)) = groups.last() {
        return Err(format!(
            "parse pattern has an unclosed '{}'",
            if *kind == '(' { ')' } else { ']' }
        )
        .into());
    }
    if let Some(reason) = unconfirmed_context {
        return Err(PatternError {
            reason,
            capability_unknown: true,
        });
    }
    Ok(placeholders)
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .replace("\"\"", "\"")
}

fn unresolved(reason: &str) -> SemanticResolution {
    SemanticResolution::Unresolved {
        reason: reason.to_owned(),
        metadata: vec![metadata("semantic-mode", "parse")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option() -> ExpressionTypeOption {
        ExpressionTypeOption {
            source_record: None,
            definition_id: "type:fixture".to_owned(),
            registration_id: "type:fixture:0".to_owned(),
            addon_name: "fixture".to_owned(),
            addon_version: "1.0.0".to_owned(),
            code_name: "number".to_owned(),
            class_name: "java.lang.Number".to_owned(),
            parser_class: None,
            type_parse_order: 0,
            before: Vec::new(),
            after: Vec::new(),
            singular: "number".to_owned(),
            plural: "numbers".to_owned(),
            user_input_patterns: Vec::new(),
            has_parser: true,
            parse_contexts: vec!["PARSE".to_owned()],
            has_supplier: false,
            default_expression: None,
        }
    }

    #[test]
    fn user_pattern_type_names_are_valid_in_parse_patterns() {
        let mut option = option();
        option.user_input_patterns = vec!["num(ber)?s?".to_owned()];
        let placeholders = parse_pattern_placeholders("value: %nums%", &[option], "PARSE")
            .expect("registered user input patterns must resolve the type");
        assert_eq!(placeholders.len(), 1);
        assert_eq!(placeholders[0].class_name, "java.lang.Number");
        assert!(placeholders[0].plural);
    }

    #[test]
    fn preserves_the_three_historical_result_shapes() {
        let single = [ParsedPlaceholder {
            class_name: "java.lang.Number".to_owned(),
            plural: false,
        }];
        assert_eq!(
            pattern_semantics(&single, false, false),
            ("java.lang.Object".to_owned(), DynamicMultiplicity::Multiple)
        );
        assert_eq!(
            pattern_semantics(&single, false, true),
            ("java.lang.Object".to_owned(), DynamicMultiplicity::Single)
        );
        assert_eq!(
            pattern_semantics(&single, true, true),
            ("java.lang.Number".to_owned(), DynamicMultiplicity::Single)
        );
    }

    #[test]
    fn plural_pattern_keeps_a_multiple_result_shape() {
        let plural = [ParsedPlaceholder {
            class_name: "java.lang.Number".to_owned(),
            plural: true,
        }];
        assert_eq!(
            pattern_semantics(&plural, true, true),
            ("java.lang.Number".to_owned(), DynamicMultiplicity::Multiple)
        );
    }

    #[test]
    fn validates_user_pattern_grouping_like_skript() {
        let options = [option()];
        assert!(parse_pattern_placeholders("(a|b) [%number%]", &options, "PARSE").is_ok());
        for invalid in ["a|b", "(a)", "(a|b", "[a", "a]", "<a", "a>"] {
            assert!(
                parse_pattern_placeholders(invalid, &options, "PARSE").is_err(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn uses_the_historical_type_parser_context() {
        assert_eq!(parse_type_context(false), "COMMAND");
        assert_eq!(parse_type_context(true), "PARSE");
    }

    #[test]
    fn missing_context_is_unconfirmed_not_a_proven_unsupported_parser() {
        let mut option = option();
        option.parse_contexts = vec!["COMMAND".to_owned()];
        let error = parse_pattern_placeholders("%number%", &[option], "PARSE")
            .err()
            .unwrap();
        assert!(error.capability_unknown);
    }

    #[test]
    fn rejects_unknown_types_after_a_known_version_is_selected() {
        assert!(parse_pattern_placeholders("%unknown%", &[], "PARSE").is_err());
    }

    #[test]
    fn ignores_escaped_and_regex_percent_signs() {
        assert!(parse_pattern_placeholders(r"\%unknown\% <%+>", &[], "PARSE").is_ok());
    }
    #[test]
    fn unconfirmed_parser_context_does_not_mask_definite_pattern_errors() {
        let mut number = option();
        number.parse_contexts.clear();
        for pattern in [
            "%number% %unknown%",
            "%number% [",
            "%number% (a)",
            "%number% <",
        ] {
            let error = parse_pattern_placeholders(pattern, &[number.clone()], "PARSE")
                .err()
                .unwrap();
            assert!(!error.capability_unknown, "{pattern}: {:?}", error.reason);
        }
        assert!(
            parse_pattern_placeholders("%number%", &[number], "PARSE")
                .err()
                .unwrap()
                .capability_unknown
        );
    }
}
