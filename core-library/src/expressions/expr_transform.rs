use super::{SemanticResolution, matches, metadata, register_handler_with_context};
use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, DynamicMultiplicity, MetadataEntry, ParseResultStatus,
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprTransform";
const HANDLER_ID: &str = "core.expression.expr-transform";
const EXPRESSION_PARSER: &str = "host.expression";
const KEY_PROVIDER: &str = "expression.capability.key-provider";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_context(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![mapping_binding()],
        Vec::new(),
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_transform(payload))
}

fn resolve_transform(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject(
            "transform Expression requires a source Expression".to_owned(),
        );
    };
    if source_is_definitely_single(source.multiplicity) {
        return SemanticResolution::Reject(
            "transform Expression source must return multiple values".to_owned(),
        );
    }
    let Some(mapping) = payload.parsed_captures.iter().find(|capture| {
        capture.capture_index == 1
            && capture.parser_id == EXPRESSION_PARSER
            && capture.status == ParseResultStatus::Success
    }) else {
        return SemanticResolution::Reject(
            "transform Expression mapping failed to parse".to_owned(),
        );
    };
    let Some(summary) = mapping.summary.as_ref() else {
        return SemanticResolution::Reject(
            "transform Expression mapping has no semantic summary".to_owned(),
        );
    };
    let Some(return_type) = summary.return_type.clone() else {
        return SemanticResolution::Reject(
            "transform Expression mapping has no return type".to_owned(),
        );
    };
    let possible_return_types = if summary.possible_return_types.is_empty() {
        vec![return_type.clone()]
    } else {
        summary.possible_return_types.clone()
    };
    let mut output_metadata = vec![metadata("semantic-mode", "transform-values")];
    let source_has_keys = source
        .metadata
        .iter()
        .any(|entry| entry.key.ends_with(KEY_PROVIDER) && entry.value == "true");
    if source_has_keys && summary.multiplicity == Some(DynamicMultiplicity::Single) {
        output_metadata.push(metadata(KEY_PROVIDER, "true"));
    }
    SemanticResolution::Resolved {
        return_type,
        possible_return_types,
        possible_return_types_state: summary.possible_return_types_state,
        multiplicity: DynamicMultiplicity::Multiple,
        metadata: output_metadata,
    }
}

fn source_is_definitely_single(multiplicity: Option<DynamicMultiplicity>) -> bool {
    multiplicity == Some(DynamicMultiplicity::Single)
}

fn mapping_binding() -> CaptureParserBinding {
    CaptureParserBinding {
        // Capture 0 is `%objects%`; capture 1 is the regex mapping body.
        capture_index: 1,
        parser_id: EXPRESSION_PARSER.to_owned(),
        required: true,
        options: vec![
            entry("expression.expected-types", "java.lang.Object[]"),
            entry("context.value.core.input-source.available", "true"),
            entry("context.value.core.input-source.has-indices", "false"),
            entry(
                "context.value.core.input-source.value-types",
                "java.lang.Object",
            ),
            entry(
                "context.value-from-child.core.input-source.has-indices",
                "0.metadata.expression.capability.key-provider",
            ),
            entry(
                "context.value-from-child.core.input-source.value-types",
                "0.possible-return-types",
            ),
        ],
    }
}

fn entry(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::source_is_definitely_single;
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    #[test]
    fn mapping_capture_is_recursively_parsed_as_an_expression() {
        let binding = super::mapping_binding();
        assert_eq!(binding.capture_index, 1);
        assert_eq!(binding.parser_id, "host.expression");
        assert!(binding.options.iter().any(|entry| {
            entry.key == "context.value-from-child.core.input-source.value-types"
                && entry.value == "0.possible-return-types"
        }));
    }

    #[test]
    fn only_definitely_single_sources_are_rejected() {
        assert!(source_is_definitely_single(Some(
            DynamicMultiplicity::Single
        )));
        assert!(!source_is_definitely_single(Some(
            DynamicMultiplicity::Multiple
        )));
        assert!(!source_is_definitely_single(Some(
            DynamicMultiplicity::Both
        )));
        assert!(!source_is_definitely_single(None));
    }
}
