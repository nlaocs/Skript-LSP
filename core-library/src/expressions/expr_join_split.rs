use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprJoinSplit";
const STRING: &str = "java.lang.String";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let Some(operation) = operation(&payload.pattern) else {
            return SemanticResolution::Reject(
                "join/split Expression has an unknown registration pattern".to_owned(),
            );
        };

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

        SemanticResolution::Resolved {
            return_type: STRING.to_owned(),
            multiplicity,
            metadata: output_metadata,
        }
    })
}

enum JoinSplitOperation {
    Join,
    Split { regex: bool },
}

// The registration text is stable across Skript 2.15.4 and 2.16.0. It also
// expresses the same distinction as ExprJoinSplit's `matchedPattern == 0`.
fn operation(pattern: &str) -> Option<JoinSplitOperation> {
    let pattern = pattern.trim_start();
    if pattern.starts_with("(concat[enate]|join)") {
        Some(JoinSplitOperation::Join)
    } else if pattern.starts_with("regex ") {
        Some(JoinSplitOperation::Split { regex: true })
    } else if pattern.starts_with("split ") || pattern.starts_with("%string% split ") {
        Some(JoinSplitOperation::Split { regex: false })
    } else {
        None
    }
}

fn has_tag(payload: &RegisteredExpressionPayload, tag: &str) -> bool {
    payload.tags.iter().any(|entry| entry.value == tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_join_and_split_from_registration_text() {
        assert!(matches!(
            operation(
                "(concat[enate]|join) %strings% [(with|using|by) [[the] delimiter] %-string%]"
            ),
            Some(JoinSplitOperation::Join)
        ));
        assert!(matches!(
            operation("split %string% (at|using|by) [[the] delimiter] %string%"),
            Some(JoinSplitOperation::Split { regex: false })
        ));
        assert!(matches!(
            operation("regex %string% split (at|using|by) [[the] delimiter] %string%"),
            Some(JoinSplitOperation::Split { regex: true })
        ));
    }

    #[test]
    fn rejects_unrelated_registration_text_without_using_pattern_index() {
        assert!(operation("join %strings% with %string%").is_none());
        assert!(operation("expression %objects%").is_none());
    }
}
