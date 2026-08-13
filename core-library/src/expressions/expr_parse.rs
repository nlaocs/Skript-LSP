use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionTypeOption, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprParse";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| resolve_parse_expression(payload))
}

fn resolve_parse_expression(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    if let Some(class_info) = payload.children.iter().find_map(|child| {
        let target = metadata_value(&child.metadata, "target-class")?;
        Some((
            target,
            metadata_value(&child.metadata, "has-parser") == Some("true"),
        ))
    }) {
        if class_info.0 == "java.lang.String" {
            return SemanticResolution::Reject("parsing text as text is not supported".to_owned());
        }
        if !class_info.1 {
            return SemanticResolution::Reject("target type has no parser".to_owned());
        }
        return SemanticResolution::Resolved {
            return_type: class_info.0.to_owned(),
            multiplicity: DynamicMultiplicity::Single,
            metadata: vec![metadata("semantic-mode", "parse-type")],
        };
    }
    let Some(pattern) = payload.regex_captures.first().map(|value| unquote(value)) else {
        return SemanticResolution::Reject("parse Expression has no static target".to_owned());
    };
    let placeholders = match parse_pattern_placeholders(&pattern, &payload.type_options) {
        Ok(placeholders) => placeholders,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let plural = placeholders.iter().any(|placeholder| placeholder.plural);
    let return_type = match placeholders.as_slice() {
        [only] => only.class_name.clone(),
        _ => "java.lang.Object".to_owned(),
    };
    SemanticResolution::Resolved {
        return_type,
        multiplicity: if placeholders.len() <= 1 && !plural {
            DynamicMultiplicity::Single
        } else {
            DynamicMultiplicity::Multiple
        },
        metadata: vec![metadata("semantic-mode", "parse-pattern")],
    }
}

struct ParsedPlaceholder {
    class_name: String,
    plural: bool,
}

fn parse_pattern_placeholders(
    pattern: &str,
    options: &[ExpressionTypeOption],
) -> Result<Vec<ParsedPlaceholder>, String> {
    let mut placeholders = Vec::new();
    let mut remaining = pattern;
    while let Some(start) = remaining.find('%') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('%') else {
            return Err("parse pattern has an unclosed type placeholder".to_owned());
        };
        let mut body = &remaining[..end];
        remaining = &remaining[end + 1..];
        body = body.trim_start_matches(['-', '*', '~']);
        if let Some((without_time, _)) = body.split_once('@') {
            body = without_time;
        }
        let alternatives = body.split('/').collect::<Vec<_>>();
        if alternatives.len() != 1 {
            placeholders.push(ParsedPlaceholder {
                class_name: "java.lang.Object".to_owned(),
                plural: alternatives
                    .iter()
                    .any(|name| type_option(name, options).is_some_and(|(_, plural)| plural)),
            });
            continue;
        }
        let (option, plural) = type_option(alternatives[0], options)
            .ok_or_else(|| format!("unknown type in parse pattern: {}", alternatives[0]))?;
        if !option.has_parser {
            return Err(format!("type has no parser: {}", option.code_name));
        }
        placeholders.push(ParsedPlaceholder {
            class_name: option.class_name.clone(),
            plural,
        });
    }
    Ok(placeholders)
}

fn type_option<'a>(
    name: &str,
    options: &'a [ExpressionTypeOption],
) -> Option<(&'a ExpressionTypeOption, bool)> {
    let name = name.trim();
    options.iter().find_map(|option| {
        if name.eq_ignore_ascii_case(&option.plural) {
            Some((option, true))
        } else if name.eq_ignore_ascii_case(&option.code_name)
            || name.eq_ignore_ascii_case(&option.singular)
        {
            Some((option, false))
        } else {
            None
        }
    })
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .replace("\"\"", "\"")
}
