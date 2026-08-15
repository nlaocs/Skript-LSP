use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler_with_all_type_options,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionTypeOption, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprParse";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_all_type_options(handlers, CLASS_SUFFIX, Vec::new());
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
                    return Err("parse pattern ends in an unescaped backslash".to_owned());
                };
                cursor += escaped.len_utf8();
            }
            '<' => {
                let Some(relative_end) = pattern[cursor + width..].find('>') else {
                    return Err("parse pattern has an unclosed regex".to_owned());
                };
                cursor += width + relative_end + 1;
            }
            '%' => {
                let body_start = cursor + width;
                let Some(relative_end) = pattern[body_start..].find('%') else {
                    return Err("parse pattern has an unclosed type placeholder".to_owned());
                };
                let body_end = body_start + relative_end;
                let body = &pattern[body_start..body_end];
                let (option, plural) = type_option(body, options)
                    .ok_or_else(|| format!("unknown type in parse pattern: {body}"))?;
                if !option.has_parser {
                    return Err(format!("type has no parser: {}", option.code_name));
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
