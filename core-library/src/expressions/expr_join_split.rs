use super::{SemanticResolution, matches, metadata, register_handler, resolved_with_metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};
use fancy_regex::Regex;

const CLASS_SUFFIX: &str = ".ExprJoinSplit";
const HANDLER_ID: &str = "core.expression.expr-join-split";
const STRING: &str = "java.lang.String";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        // ExprJoinSplit.init() uses matchedPattern == 0 for join and >= 3 for regex split.
        // The five pattern indices are stable from Skript 2.6.4 through 2.16.x.
        let Some(operation) = operation(payload.pattern_index) else {
            return SemanticResolution::Reject(
                "join/split Expression has an unknown pattern index".to_owned(),
            );
        };
        if let Err(reason) = validate_literal_regex_delimiter(
            matches!(operation, JoinSplitOperation::Split { regex: true }),
            payload.children.get(1),
        ) {
            return SemanticResolution::Reject(reason);
        }

        let (operation_name, multiplicity, regex) = match operation {
            JoinSplitOperation::Join => ("join", DynamicMultiplicity::Single, false),
            JoinSplitOperation::Split { regex } => ("split", DynamicMultiplicity::Multiple, regex),
        };

        let mut output_metadata = vec![
            metadata("semantic-mode", "join-split"),
            metadata("operation", operation_name),
            metadata("regex", if regex { "true" } else { "false" }),
        ];
        if has_tag(payload, "case") {
            output_metadata.push(metadata("explicit-case-sensitive", "true"));
        }
        if has_tag(payload, "trailing") {
            output_metadata.push(metadata("without-trailing-empty", "true"));
        }

        resolved_with_metadata(STRING.to_owned(), multiplicity, output_metadata)
    })
}

#[derive(Clone, Copy)]
enum JoinSplitOperation {
    Join,
    Split { regex: bool },
}

fn validate_literal_regex_delimiter(
    regex: bool,
    delimiter: Option<&RegisteredExpressionChild>,
) -> Result<(), String> {
    if !regex {
        return Ok(());
    }
    let Some(delimiter) = delimiter
        .filter(|child| child.kind == "literal" && child.return_type.as_deref() == Some(STRING))
    else {
        return Ok(());
    };
    let Some(value) = decode_string_literal(&delimiter.text) else {
        return Ok(());
    };
    Regex::new(&value)
        .map(|_| ())
        .map_err(|_| format!("'{value}' is not a valid regular expression"))
}

fn decode_string_literal(value: &str) -> Option<String> {
    let value = value.trim();
    Some(
        value
            .strip_prefix('"')?
            .strip_suffix('"')?
            .replace("\"\"", "\""),
    )
}

fn operation(pattern_index: u64) -> Option<JoinSplitOperation> {
    match pattern_index {
        0 => Some(JoinSplitOperation::Join),
        1 | 2 => Some(JoinSplitOperation::Split { regex: false }),
        3 | 4 => Some(JoinSplitOperation::Split { regex: true }),
        _ => None,
    }
}

fn has_tag(payload: &RegisteredExpressionPayload, tag: &str) -> bool {
    payload.tags.iter().any(|entry| entry.value == tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState;

    fn delimiter(text: &str, literal: bool) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            default_expression: None,
            text: text.to_owned(),
            kind: if literal { "literal" } else { "variable" }.to_owned(),
            parser_id: literal.then(|| "core.literal.string".to_owned()),
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some(STRING.to_owned()),
            possible_return_types: vec![STRING.to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn classifies_join_and_split_from_skript_pattern_index() {
        assert!(matches!(operation(0), Some(JoinSplitOperation::Join)));
        for pattern_index in [1, 2] {
            assert!(matches!(
                operation(pattern_index),
                Some(JoinSplitOperation::Split { regex: false })
            ));
        }
        for pattern_index in [3, 4] {
            assert!(matches!(
                operation(pattern_index),
                Some(JoinSplitOperation::Split { regex: true })
            ));
        }
    }

    #[test]
    fn rejects_an_unknown_pattern_index() {
        assert!(operation(5).is_none());
    }

    #[test]
    fn rejects_an_invalid_literal_regex_delimiter() {
        assert!(validate_literal_regex_delimiter(true, Some(&delimiter("\"[\"", true))).is_err());
    }

    #[test]
    fn leaves_dynamic_and_plain_delimiters_for_runtime_evaluation() {
        assert!(
            validate_literal_regex_delimiter(true, Some(&delimiter("{_delimiter}", false))).is_ok()
        );
        assert!(validate_literal_regex_delimiter(false, Some(&delimiter("\"[\"", true))).is_ok());
    }
}
