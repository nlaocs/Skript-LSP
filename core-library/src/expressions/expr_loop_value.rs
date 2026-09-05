use super::{
    SemanticResolution, matches, metadata, register_handler_with_all_type_options,
    resolved_with_possible_types,
};
use crate::catalog::TypeRelation;
use crate::loop_context::LoopFrame;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprLoopValue";
const HANDLER_ID: &str = "core.expression.expr-loop-value";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_all_type_options(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_loop_value(payload))
}

fn resolve_loop_value(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(raw_name) = payload.regex_captures.first() else {
        return SemanticResolution::Reject("loop value has no loop name".to_owned());
    };
    let (name, expected_depth) = split_depth(raw_name);
    if matches!(name.to_ascii_lowercase().as_str(), "counter" | "iteration") {
        return SemanticResolution::Reject(
            "loop counter and loop iteration use a different Expression".to_owned(),
        );
    }
    let frames =
        crate::loop_context::decode(context_value(payload, crate::loop_context::CONTEXT_KEY));
    if frames.is_empty() {
        return SemanticResolution::Reject(format!("there is no loop matching loop-{name}"));
    }

    let mut candidates = Vec::new();
    let mut unresolved = false;
    // Skript numbers matching loops from the outermost active loop. The
    // unnumbered form remains ambiguous when more than one loop has the same
    // value type.
    for frame in &frames {
        match frame_matches(name, frame, payload) {
            Ok(true) => candidates.push(frame),
            Ok(false) => {}
            Err(()) => unresolved = true,
        }
    }
    let selected = match candidate_index(candidates.len(), expected_depth) {
        Ok(Some(index)) => candidates.get(index).copied(),
        Ok(None) => None,
        Err(()) => {
            return SemanticResolution::Reject(format!(
                "multiple loops match loop-{name}; add -1, -2, and so on"
            ));
        }
    };
    let Some(frame) = selected else {
        if unresolved {
            return SemanticResolution::Unresolved {
                reason: format!("loop-{name} could not be matched because type data is incomplete"),
                metadata: vec![metadata("semantic-mode", "loop-value")],
            };
        }
        return SemanticResolution::Reject(format!("there is no loop matching loop-{name}"));
    };
    if payload.pattern_index == 1 && frame.supports_peeking == Some(false) {
        return SemanticResolution::Reject(format!("this loop does not support next loop-{name}"));
    }

    let index = name.eq_ignore_ascii_case("index");
    let return_type = if index {
        "java.lang.String".to_owned()
    } else {
        frame.return_type.clone()
    };
    let possible_return_types = if index || frame.possible_return_types.is_empty() {
        vec![return_type.clone()]
    } else {
        frame.possible_return_types.clone()
    };
    let mut output_metadata = vec![
        metadata("semantic-mode", "loop-value"),
        metadata(
            "loop-state",
            match payload.pattern_index {
                0 => "current",
                1 => "next",
                2 => "previous",
                _ => "unknown",
            },
        ),
    ];
    if payload.pattern_index == 1 && frame.supports_peeking.is_none() {
        output_metadata.push(metadata("loop-peeking-state", "unresolved"));
    }
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        ExpressionPossibleReturnTypesState::Complete,
        DynamicMultiplicity::Single,
        output_metadata,
    )
}

fn frame_matches(
    name: &str,
    frame: &LoopFrame,
    payload: &RegisteredExpressionPayload,
) -> Result<bool, ()> {
    if name.eq_ignore_ascii_case("value") {
        return Ok(true);
    }
    if name.eq_ignore_ascii_case("index") {
        return frame.keyed.ok_or(());
    }
    let Some((option, _)) = crate::types::match_type_option(name, &payload.type_options) else {
        return Ok(false);
    };
    let mut unknown = false;
    for class_name in frame
        .possible_return_types
        .iter()
        .chain(std::iter::once(&frame.return_type))
    {
        match crate::catalog::is_class_assignable(class_name, &option.class_name) {
            Ok(TypeRelation::Compatible) => return Ok(true),
            Ok(TypeRelation::Incompatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => unknown = true,
        }
    }
    if unknown { Err(()) } else { Ok(false) }
}

fn context_value<'a>(payload: &'a RegisteredExpressionPayload, key: &str) -> Option<&'a str> {
    payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

fn split_depth(value: &str) -> (&str, Option<usize>) {
    let value = value.trim();
    let Some((name, depth)) = value.rsplit_once('-') else {
        return (value, None);
    };
    match depth.parse::<usize>().ok() {
        Some(depth) => (name, Some(depth)),
        None => (value, None),
    }
}

fn candidate_index(count: usize, expected_depth: Option<usize>) -> Result<Option<usize>, ()> {
    match expected_depth {
        Some(depth @ 1..) => Ok((depth <= count).then_some(depth - 1)),
        Some(0) | None => match count {
            0 => Ok(None),
            1 => Ok(Some(0)),
            _ => Err(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{candidate_index, split_depth};

    #[test]
    fn loop_depth_suffix_is_optional_and_accepts_skripts_zero_depth() {
        assert_eq!(split_depth("player"), ("player", None));
        assert_eq!(split_depth("player-0"), ("player", Some(0)));
        assert_eq!(split_depth("player-2"), ("player", Some(2)));
        assert_eq!(split_depth("player-zero"), ("player-zero", None));
    }

    #[test]
    fn zero_depth_uses_skripts_unnumbered_ambiguity_rules() {
        assert_eq!(candidate_index(1, Some(0)), Ok(Some(0)));
        assert_eq!(candidate_index(2, Some(0)), Err(()));
        assert_eq!(candidate_index(2, Some(2)), Ok(Some(1)));
    }
}
