use super::{
    condition_binding, condition_captures_are_parsed, continue_with_section_context,
    parse_condition_binding, register_handler, register_pattern_handler, reject_section,
    request_parses,
};
use crate::nlaocs::skript_parser_addon::types::{
    ContextUpdate, HookOutput, InvocationContext, MetadataEntry, ParseRequest, ParseResult,
    ParseResultStatus, RegisteredSyntaxHandler, SectionBodyMode, SectionPayload,
    SectionRawNodeKind, SectionSibling, SectionTiming,
};

const CLASS_SUFFIX: &str = ".SecConditional";
const HANDLER_ID: &str = "core.section.sec-conditional";
const CONDITION_HANDLER_ID: &str = "core.section.sec-conditional.condition";
const PARSE_CONDITION_HANDLER_ID: &str = "core.section.sec-conditional.parse-condition";
const LEGACY_CONDITION_HANDLER_ID: &str = "core.section.sec-conditional.legacy-condition";
const LEGACY_PARSE_CONDITION_HANDLER_ID: &str =
    "core.section.sec-conditional.legacy-parse-condition";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
    register_pattern_handler(
        handlers,
        CONDITION_HANDLER_ID,
        CLASS_SUFFIX,
        ["else [:parse] if <.+>", "[:parse] if <.+>", "implicit:<.+>"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        Vec::new(),
        vec!["parse".to_owned()],
        Vec::new(),
        vec![condition_binding()],
    );
    register_pattern_handler(
        handlers,
        PARSE_CONDITION_HANDLER_ID,
        CLASS_SUFFIX,
        ["else [:parse] if <.+>", "[:parse] if <.+>"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        vec!["parse".to_owned()],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let legacy_patterns = [
        "else [(1¦parse)] if <.+>".to_owned(),
        "[(1¦parse if|2¦if)] <.+>".to_owned(),
    ];
    register_pattern_handler(
        handlers,
        LEGACY_CONDITION_HANDLER_ID,
        CLASS_SUFFIX,
        legacy_patterns.to_vec(),
        Vec::new(),
        Vec::new(),
        vec![0, 2],
        vec![condition_binding()],
    );
    register_pattern_handler(
        handlers,
        LEGACY_PARSE_CONDITION_HANDLER_ID,
        CLASS_SUFFIX,
        legacy_patterns.to_vec(),
        Vec::new(),
        Vec::new(),
        vec![1],
        vec![parse_condition_binding(
            "ch.njol.skript.events.bukkit.SkriptParseEvent",
        )],
    );
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionalKind {
    Else,
    ElseIf,
    If,
    Then,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConditionalSemantics {
    kind: ConditionalKind,
    parse_if: bool,
    if_any: bool,
    multiline: bool,
    expected_conditions: usize,
    implicit: bool,
}

pub(super) fn resolve(
    context: InvocationContext,
    mut payload: SectionPayload,
    parse_results: &[ParseResult],
) -> HookOutput {
    // SecConditional switched from the three 2.6 ParseMark patterns to the
    // nine tag-based patterns in Skript 2.7.0. This is earlier than the
    // 2.16.1 source used as the modern reference point.
    let Some(modern) = crate::runtime::skript_at_least(2, 7) else {
        return unresolved_version(context, payload);
    };
    let semantics = match classify(
        modern,
        payload.candidate.pattern_index,
        has_tag(&payload, "parse"),
        has_tag(&payload, "any"),
        has_tag(&payload, "implicit"),
        payload.candidate.mark,
        payload.candidate.regex_captures.len(),
    ) {
        Ok(semantics) => semantics,
        Err(reason) => return reject_section(reason),
    };
    if semantics.parse_if && crate::runtime::skript_at_least(2, 9).is_none() {
        return unresolved_version(context, payload);
    }

    let mark_values = payload
        .candidate
        .marks
        .iter()
        .map(|capture| capture.value)
        .collect::<Vec<_>>();
    if let Err(reason) = validate_mark_captures(
        modern,
        payload.candidate.pattern_index,
        payload.candidate.mark,
        &mark_values,
    ) {
        return reject_section(reason);
    }

    let entering = matches!(payload.timing, SectionTiming::EnterChildren);
    if semantics.parse_if && modern {
        if payload.candidate.regex_captures.len() != semantics.expected_conditions {
            return reject_section("parse-if Section has an invalid Condition capture count");
        }
        if entering {
            let Some(requests) = inline_parse_condition_requests(&payload) else {
                return unresolved_version(context, payload);
            };
            if parse_results.is_empty() {
                return request_parses(payload, requests);
            }
            if let Err(reason) = validate_condition_results(&requests, parse_results) {
                return reject_section(reason);
            }
        }
    } else if let Err(reason) =
        condition_captures_are_parsed(&payload, semantics.expected_conditions)
    {
        return reject_section(reason);
    }

    if entering {
        if let Err(reason) = validate_conditional_chain(&payload, semantics) {
            return reject_section(reason);
        }
        if semantics.multiline {
            let requests = match multiline_condition_requests(&payload, semantics) {
                Ok(requests) => requests,
                Err(reason) => return reject_section(reason),
            };
            if parse_results.is_empty() {
                return request_parses(payload, requests);
            }
            if let Err(reason) = validate_condition_results(&requests, parse_results) {
                return reject_section(reason);
            }
            payload.candidate.body_mode = SectionBodyMode::Conditions;
        } else {
            payload.candidate.body_mode = SectionBodyMode::Trigger;
        }
    }

    let chain_delay_before = chain_delay_before(&payload, semantics.kind);
    let branch_delay = branch_delay_state(&payload, chain_delay_before, entering);
    let mut updates = vec![ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.conditional.kind".to_owned(),
        value: Some(kind_name(semantics.kind).as_bytes().to_vec()),
    }];
    updates.push(ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.conditional.parse-if".to_owned(),
        value: Some(if semantics.parse_if {
            b"true".to_vec()
        } else {
            b"false".to_vec()
        }),
    });
    if entering {
        updates.push(super::increment_context_update(
            &payload.context,
            context.syntax_context,
            "core.section.conditional-depth",
        ));
        updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.delay-state".to_owned(),
            value: Some(branch_delay.as_bytes().to_vec()),
        });
    } else if !semantics.multiline || semantics.kind == ConditionalKind::Then {
        let (merged, after) = merged_delay_after(&payload, semantics.kind, chain_delay_before);
        payload.candidate.metadata.extend([
            metadata_entry("conditional-should-delay-after", merged),
            metadata_entry("conditional-delay-after", after),
        ]);
        updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.delay-state".to_owned(),
            value: Some(after.as_bytes().to_vec()),
        });
    }
    updates.push(ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.conditional.multiline".to_owned(),
        value: Some(if semantics.multiline {
            b"true".to_vec()
        } else {
            b"false".to_vec()
        }),
    });
    // The registered capture binding gives only the `parse if` Condition a
    // SkriptParseEvent/ContextlessEvent context. SecConditional restores the
    // surrounding Event before parsing the section body, so no Event context
    // update belongs on the child scope here.
    if semantics.multiline {
        updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.section.conditional.if-mode".to_owned(),
            value: Some(if semantics.if_any { b"any" } else { b"all" }.to_vec()),
        });
    }

    let metadata = [
        ("semantic-mode", "conditional".to_owned()),
        ("conditional-kind", kind_name(semantics.kind).to_owned()),
        ("conditional-parse-if", semantics.parse_if.to_string()),
        ("conditional-multiline", semantics.multiline.to_string()),
        (
            "conditional-condition-count",
            semantics.expected_conditions.to_string(),
        ),
        ("conditional-implicit", semantics.implicit.to_string()),
        ("conditional-delay-before", chain_delay_before.to_owned()),
        (
            "conditional-chain-validation",
            "host-sibling-context-required".to_owned(),
        ),
    ];
    continue_with_section_context(&context, payload, metadata, updates)
}

fn chain_delay_before(payload: &SectionPayload, kind: ConditionalKind) -> &'static str {
    if kind == ConditionalKind::If {
        return delay_state(&payload.context);
    }
    payload
        .preceding_siblings
        .iter()
        .rev()
        .take_while(|sibling| is_conditional(sibling) && sibling_kind(sibling) != Some("else"))
        .find_map(|sibling| {
            (sibling_kind(sibling) == Some("if"))
                .then(|| sibling_metadata(sibling, "conditional-delay-before"))
                .flatten()
        })
        .and_then(normalize_delay_state)
        .unwrap_or_else(|| delay_state(&payload.context))
}

fn branch_delay_state(
    payload: &SectionPayload,
    chain_delay_before: &'static str,
    entering: bool,
) -> &'static str {
    if !entering || delay_state(&payload.context) == "true" {
        delay_state(&payload.context)
    } else {
        chain_delay_before
    }
}

fn merged_delay_after(
    payload: &SectionPayload,
    kind: ConditionalKind,
    chain_delay_before: &'static str,
) -> (&'static str, &'static str) {
    let branch_after = delay_state(&payload.context);
    let preceding = payload
        .preceding_siblings
        .iter()
        .rev()
        .take_while(|sibling| is_conditional(sibling) && sibling_kind(sibling) != Some("else"))
        .find_map(|sibling| sibling_metadata(sibling, "conditional-should-delay-after"))
        .and_then(normalize_delay_state);
    merge_delay_states(kind, chain_delay_before, branch_after, preceding)
}

fn merge_delay_states(
    kind: ConditionalKind,
    chain_delay_before: &'static str,
    branch_after: &'static str,
    preceding: Option<&'static str>,
) -> (&'static str, &'static str) {
    if chain_delay_before == "true" {
        return ("true", "true");
    }
    let merged = match preceding {
        None => branch_after,
        Some(value) if value == branch_after => branch_after,
        Some(_) => "unknown",
    };
    let after = if merged == "true" && kind != ConditionalKind::Else {
        "unknown"
    } else {
        merged
    };
    (merged, after)
}

fn delay_state(context: &crate::nlaocs::skript_parser_addon::types::ParseContext) -> &'static str {
    context
        .values
        .iter()
        .rfind(|entry| entry.key == "parser.delay-state")
        .and_then(|entry| normalize_delay_state(&entry.value))
        .unwrap_or("unknown")
}

fn normalize_delay_state(value: &str) -> Option<&'static str> {
    match value {
        "true" => Some("true"),
        "false" => Some("false"),
        "unknown" => Some("unknown"),
        _ => None,
    }
}

fn inline_parse_condition_requests(payload: &SectionPayload) -> Option<Vec<ParseRequest>> {
    let options = condition_parse_options(&payload.context, true)?;
    payload
        .candidate
        .regex_captures
        .iter()
        .enumerate()
        .map(|(request_id, source)| {
            Some(ParseRequest {
                request_id: request_id as u64,
                parser_id: super::CONDITION_PARSER_ID.to_owned(),
                input: source.clone(),
                expected_types: Vec::new(),
                span: payload.candidate.span.clone(),
                options: options.clone(),
            })
        })
        .collect()
}

/// Mirrors the sibling walk in `SecConditional.getPrecedingConditional`.
///
/// The native parser supplies only contiguous parsed Section siblings. CoreLibrary
/// still stops when it encounters a non-conditional Section, matching Java's early
/// return when the preceding TriggerItem is not `SecConditional`.
fn validate_conditional_chain(
    payload: &SectionPayload,
    semantics: ConditionalSemantics,
) -> Result<(), String> {
    match semantics.kind {
        ConditionalKind::If => {
            if semantics.multiline && !is_then_header(payload.next_sibling.as_ref()) {
                return Err(format!(
                    "'if {}' has to be placed just before a 'then' section",
                    if semantics.if_any { "any" } else { "all" }
                ));
            }
        }
        ConditionalKind::Else | ConditionalKind::ElseIf => {
            if preceding_if(&payload.preceding_siblings).is_none() {
                return Err(format!(
                    "'{}' has to be placed just after another 'if' or 'else if' section",
                    kind_name(semantics.kind)
                ));
            }
        }
        ConditionalKind::Then => {
            let Some(previous) = payload.preceding_siblings.last() else {
                return Err(
                    "'then' has to be placed just after a multiline 'if' or 'else if' section"
                        .to_owned(),
                );
            };
            let kind = sibling_kind(previous);
            let multiline = sibling_metadata(previous, "conditional-multiline") == Some("true");
            let repeated_then =
                crate::runtime::skript_at_least(2, 15) == Some(true) && kind == Some("then");
            if !is_conditional(previous) || !multiline || repeated_then {
                return Err(
                    "'then' has to be placed just after a multiline 'if' or 'else if' section"
                        .to_owned(),
                );
            }
        }
    }
    Ok(())
}

fn preceding_if(siblings: &[SectionSibling]) -> Option<&SectionSibling> {
    for sibling in siblings.iter().rev() {
        if !is_conditional(sibling) || sibling_kind(sibling) == Some("else") {
            return None;
        }
        if sibling_kind(sibling) == Some("if") {
            return Some(sibling);
        }
    }
    None
}

fn is_conditional(sibling: &SectionSibling) -> bool {
    sibling
        .element_class
        .as_deref()
        .is_some_and(|class| class.ends_with(CLASS_SUFFIX))
        || sibling_metadata(sibling, "semantic-mode") == Some("conditional")
}

fn sibling_kind(sibling: &SectionSibling) -> Option<&str> {
    sibling_metadata(sibling, "conditional-kind")
}

fn sibling_metadata<'a>(sibling: &'a SectionSibling, key: &str) -> Option<&'a str> {
    sibling
        .metadata
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

fn is_then_header(
    next: Option<&crate::nlaocs::skript_parser_addon::types::SectionRawNode>,
) -> bool {
    let Some(next) = next.filter(|next| matches!(next.kind, SectionRawNodeKind::Section)) else {
        return false;
    };
    let header = next
        .source
        .trim()
        .strip_suffix(':')
        .unwrap_or(next.source.trim());
    header.eq_ignore_ascii_case("then") || header.eq_ignore_ascii_case("then run")
}

fn multiline_condition_requests(
    payload: &SectionPayload,
    semantics: ConditionalSemantics,
) -> Result<Vec<ParseRequest>, String> {
    let options =
        condition_parse_options(&payload.context, semantics.parse_if).ok_or_else(|| {
            "Skript version is unavailable, so conditional parsing is unresolved".to_owned()
        })?;
    let children = payload
        .raw_children
        .iter()
        .filter(|child| {
            !matches!(
                child.kind,
                SectionRawNodeKind::Blank | SectionRawNodeKind::Comment
            )
        })
        .collect::<Vec<_>>();
    if children.len() < 2 {
        return Err(format!(
            "'if {}' sections must contain at least two conditions",
            if semantics.if_any { "any" } else { "all" }
        ));
    }
    if children
        .iter()
        .any(|child| matches!(child.kind, SectionRawNodeKind::Section))
    {
        return Err(format!(
            "'if {}' sections may not contain other sections",
            if semantics.if_any { "any" } else { "all" }
        ));
    }
    if children
        .iter()
        .any(|child| !matches!(child.kind, SectionRawNodeKind::Simple))
    {
        return Err("multiline conditional contains an invalid condition line".to_owned());
    }

    Ok(children
        .into_iter()
        .enumerate()
        .map(|(request_id, child)| ParseRequest {
            request_id: request_id as u64,
            parser_id: super::CONDITION_PARSER_ID.to_owned(),
            input: child.source.clone(),
            expected_types: Vec::new(),
            span: child.span.clone(),
            options: options.clone(),
        })
        .collect())
}

fn condition_parse_options(
    context: &crate::nlaocs::skript_parser_addon::types::ParseContext,
    parse_if: bool,
) -> Option<Vec<MetadataEntry>> {
    let mut options = crate::structures::parse_context_options(context);
    if parse_if {
        let event_class = parse_if_event_class(crate::runtime::skript_at_least(2, 9)?);
        options.push(metadata_entry("context.event-classes", event_class));
        options.push(metadata_entry("context.value.parser.event-name", "parse"));
    }
    Some(options)
}

fn parse_if_event_class(modern: bool) -> &'static str {
    if modern {
        "ch.njol.skript.lang.util.ContextlessEvent"
    } else {
        "ch.njol.skript.events.bukkit.SkriptParseEvent"
    }
}

fn validate_condition_results(
    requests: &[ParseRequest],
    results: &[ParseResult],
) -> Result<(), String> {
    if requests.len() != results.len() {
        return Err(
            "multiline condition parse results do not match the requested lines".to_owned(),
        );
    }
    for request in requests {
        let Some(result) = results.iter().find(|result| {
            result.request_id == request.request_id && result.parser_id == request.parser_id
        }) else {
            return Err(format!(
                "condition result is missing for `{}`",
                request.input
            ));
        };
        if result.status != ParseResultStatus::Success || result.roots.is_empty() {
            return Err(format!(
                "can't understand this condition: '{}'",
                request.input
            ));
        }
    }
    Ok(())
}

fn metadata_entry(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

/// Mirrors `SecConditional.CONDITIONAL_PATTERNS` and `init` from Skript.
///
/// The modern registration has nine patterns (including multiline `if any/all`
/// and `then`). The 2.6.4 registration has only three patterns; its `parse if`
/// choice is a ParseMark rather than the modern `parse` tag. This function is
/// intentionally independent of the host payload so the mapping is testable.
fn classify(
    modern: bool,
    pattern_index: u64,
    parse_tag: bool,
    any_tag: bool,
    implicit_tag: bool,
    mark: i32,
    regex_capture_count: usize,
) -> Result<ConditionalSemantics, String> {
    if modern {
        if mark != 0 {
            return Err("2.16 SecConditional cannot carry a ParseMark".to_owned());
        }
        let semantics = match pattern_index {
            0 => ConditionalSemantics {
                kind: ConditionalKind::Else,
                parse_if: false,
                if_any: false,
                multiline: false,
                expected_conditions: 0,
                implicit: false,
            },
            1 => ConditionalSemantics {
                kind: ConditionalKind::ElseIf,
                parse_if: parse_tag,
                if_any: false,
                multiline: false,
                expected_conditions: 1,
                implicit: false,
            },
            2 => ConditionalSemantics {
                kind: ConditionalKind::ElseIf,
                parse_if: parse_tag,
                if_any: true,
                multiline: true,
                expected_conditions: 0,
                implicit: false,
            },
            3 => ConditionalSemantics {
                kind: ConditionalKind::ElseIf,
                parse_if: parse_tag,
                if_any: false,
                multiline: true,
                expected_conditions: 0,
                implicit: false,
            },
            4 => ConditionalSemantics {
                kind: ConditionalKind::If,
                parse_if: parse_tag,
                if_any: true,
                multiline: true,
                expected_conditions: 0,
                implicit: false,
            },
            5 => ConditionalSemantics {
                kind: ConditionalKind::If,
                parse_if: parse_tag,
                if_any: false,
                multiline: true,
                expected_conditions: 0,
                implicit: false,
            },
            6 => ConditionalSemantics {
                kind: ConditionalKind::If,
                parse_if: parse_tag,
                if_any: false,
                multiline: false,
                expected_conditions: 1,
                implicit: false,
            },
            7 => ConditionalSemantics {
                kind: ConditionalKind::Then,
                parse_if: false,
                if_any: false,
                multiline: false,
                expected_conditions: 0,
                implicit: false,
            },
            8 => ConditionalSemantics {
                kind: ConditionalKind::If,
                parse_if: false,
                if_any: false,
                multiline: false,
                expected_conditions: 1,
                implicit: true,
            },
            _ => {
                return Err(format!(
                    "SecConditional has unknown 2.16 pattern index {pattern_index}"
                ));
            }
        };
        let expected_any_tag = matches!(pattern_index, 2 | 4);
        if any_tag != expected_any_tag {
            return Err(format!(
                "SecConditional pattern {pattern_index} has an inconsistent any tag"
            ));
        }
        if semantics.multiline && regex_capture_count != 0 {
            return Err(
                "multiline if any/all must not contain a header condition capture".to_owned(),
            );
        }
        if !semantics.multiline && regex_capture_count != semantics.expected_conditions {
            return Err(
                "single-line conditional has an inconsistent condition capture count".to_owned(),
            );
        }
        if semantics.kind == ConditionalKind::Then && (parse_tag || implicit_tag) {
            return Err("then cannot carry parse or implicit tags".to_owned());
        }
        let expected_parse_tag = matches!(pattern_index, 1..=6);
        if parse_tag && !expected_parse_tag {
            return Err(format!(
                "SecConditional pattern {pattern_index} cannot carry the parse tag"
            ));
        }
        if implicit_tag != (pattern_index == 8) {
            return Err(format!(
                "SecConditional pattern {pattern_index} has an inconsistent implicit tag"
            ));
        }
        if semantics.kind != ConditionalKind::If && pattern_index != 8 && implicit_tag {
            return Err(
                "only the implicit conditional pattern may carry the implicit tag".to_owned(),
            );
        }
        return Ok(semantics);
    }

    if parse_tag || any_tag || implicit_tag {
        return Err(
            "2.6 SecConditional uses ParseMarks and has no modern conditional tags".to_owned(),
        );
    }
    let semantics = match pattern_index {
        0 if mark == 0 => ConditionalSemantics {
            kind: ConditionalKind::Else,
            parse_if: false,
            if_any: false,
            multiline: false,
            expected_conditions: 0,
            implicit: false,
        },
        1 if matches!(mark, 0 | 1) => ConditionalSemantics {
            kind: ConditionalKind::ElseIf,
            parse_if: mark == 1,
            if_any: false,
            multiline: false,
            expected_conditions: 1,
            implicit: false,
        },
        2 if matches!(mark, 0..=2) => ConditionalSemantics {
            kind: ConditionalKind::If,
            parse_if: mark == 1,
            if_any: false,
            multiline: false,
            expected_conditions: 1,
            implicit: false,
        },
        _ => {
            return Err(format!(
                "SecConditional has unknown 2.6 pattern index {pattern_index}"
            ));
        }
    };
    if mark != 0 && mark != 1 && mark != 2 {
        return Err(format!(
            "SecConditional has an invalid legacy parse mark {mark}"
        ));
    }
    Ok(semantics)
}

fn validate_mark_captures(
    modern: bool,
    pattern_index: u64,
    mark: i32,
    marks: &[i32],
) -> Result<(), String> {
    if modern {
        if !marks.is_empty() || mark != 0 {
            return Err("2.16 SecConditional cannot carry a ParseMark".to_owned());
        }
        return Ok(());
    }
    let allowed = match pattern_index {
        0 => &[][..],
        1 => &[1][..],
        2 => &[1, 2][..],
        _ => return Ok(()),
    };
    if marks.iter().any(|value| !allowed.contains(value)) {
        return Err(format!(
            "2.6 SecConditional pattern {pattern_index} has an invalid ParseMark capture"
        ));
    }
    if !allowed.contains(&mark) && mark != 0 {
        return Err(format!(
            "SecConditional has an invalid legacy parse mark {mark}"
        ));
    }
    Ok(())
}

fn unresolved_version(context: InvocationContext, payload: SectionPayload) -> HookOutput {
    let span = payload.candidate.span.clone();
    let mut output = continue_with_section_context(
        &context,
        payload,
        [
            ("semantic-mode", "conditional".to_owned()),
            ("semantic-state", "unresolved".to_owned()),
        ],
        Vec::new(),
    );
    output.effects.diagnostics.push(super::warning(
        "core.section.conditional.unresolved-version",
        "the Skript version is unavailable, so conditional syntax semantics were not selected",
        span,
    ));
    output
}

fn has_tag(payload: &SectionPayload, value: &str) -> bool {
    payload.candidate.tags.iter().any(|tag| tag.value == value)
}

fn kind_name(kind: ConditionalKind) -> &'static str {
    match kind {
        ConditionalKind::Else => "else",
        ConditionalKind::ElseIf => "else-if",
        ConditionalKind::If => "if",
        ConditionalKind::Then => "then",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConditionalKind, classify, is_then_header, merge_delay_states, parse_if_event_class,
        preceding_if,
    };
    use crate::nlaocs::skript_parser_addon::types::{
        MappedSpan, MetadataEntry, SectionRawNode, SectionRawNodeKind, SectionSibling, TextRange,
    };

    fn span() -> MappedSpan {
        MappedSpan {
            virtual_range: TextRange { start: 0, end: 1 },
            origins: Vec::new(),
        }
    }

    fn sibling(kind: &str, multiline: bool) -> SectionSibling {
        SectionSibling {
            raw_node_id: 1,
            definition_id: "section:skript:conditional".to_owned(),
            registration_id: format!("section:skript:conditional:{kind}"),
            element_class: Some("ch.njol.skript.sections.SecConditional".to_owned()),
            pattern_index: 0,
            source: kind.to_owned(),
            span: span(),
            handler: None,
            metadata: vec![
                MetadataEntry {
                    key: "semantic-mode".to_owned(),
                    value: "conditional".to_owned(),
                    owner_component_id: None,
                },
                MetadataEntry {
                    key: "conditional-kind".to_owned(),
                    value: kind.to_owned(),
                    owner_component_id: None,
                },
                MetadataEntry {
                    key: "conditional-multiline".to_owned(),
                    value: multiline.to_string(),
                    owner_component_id: None,
                },
            ],
        }
    }

    #[test]
    fn maps_modern_conditional_patterns_and_tags() {
        assert_eq!(
            classify(true, 0, false, false, false, 0, 0).unwrap().kind,
            ConditionalKind::Else
        );
        let else_if = classify(true, 1, true, false, false, 0, 1).unwrap();
        assert_eq!(else_if.kind, ConditionalKind::ElseIf);
        assert!(else_if.parse_if);
        let any = classify(true, 4, true, true, false, 0, 0).unwrap();
        assert!(any.multiline);
        assert!(any.if_any);
        assert_eq!(
            classify(true, 7, false, false, false, 0, 0).unwrap().kind,
            ConditionalKind::Then
        );
        assert!(
            classify(true, 8, false, false, true, 0, 1)
                .unwrap()
                .implicit
        );
    }

    #[test]
    fn covers_every_modern_pattern_shape() {
        let expected = [
            (ConditionalKind::Else, false, false, false, 0),
            (ConditionalKind::ElseIf, true, false, false, 1),
            (ConditionalKind::ElseIf, false, true, true, 0),
            (ConditionalKind::ElseIf, true, false, true, 0),
            (ConditionalKind::If, false, true, true, 0),
            (ConditionalKind::If, true, false, true, 0),
            (ConditionalKind::If, false, false, false, 1),
            (ConditionalKind::Then, false, false, false, 0),
            (ConditionalKind::If, false, false, false, 1),
        ];
        for (pattern_index, (kind, parse_if, if_any, multiline, conditions)) in
            expected.into_iter().enumerate()
        {
            let parse_tag = matches!(pattern_index, 1..=6) && parse_if;
            let any_tag = matches!(pattern_index, 2 | 4);
            let implicit_tag = pattern_index == 8;
            let regex_capture_count = if multiline { 0 } else { conditions };
            let semantics = classify(
                true,
                pattern_index as u64,
                parse_tag,
                any_tag,
                implicit_tag,
                0,
                regex_capture_count,
            )
            .expect("every modern registration pattern should classify");
            assert_eq!(semantics.kind, kind);
            assert_eq!(semantics.parse_if, parse_if);
            assert_eq!(semantics.if_any, if_any);
            assert_eq!(semantics.multiline, multiline);
            assert_eq!(semantics.expected_conditions, conditions);
        }
    }

    #[test]
    fn rejects_inconsistent_modern_tags() {
        assert!(classify(true, 0, true, false, false, 0, 0).is_err());
        assert!(classify(true, 4, false, false, false, 0, 0).is_err());
        assert!(classify(true, 7, true, false, false, 0, 0).is_err());
        assert!(classify(true, 8, false, false, false, 0, 1).is_err());
    }

    #[test]
    fn maps_legacy_parse_marks_without_inventing_modern_patterns() {
        assert!(
            classify(false, 1, false, false, false, 1, 1)
                .unwrap()
                .parse_if
        );
        assert!(
            !classify(false, 2, false, false, false, 2, 1)
                .unwrap()
                .parse_if
        );
        assert!(classify(false, 2, false, false, false, 0, 1).is_ok());
        assert!(classify(false, 3, false, false, false, 0, 0).is_err());
        assert!(classify(false, 2, false, true, false, 0, 1).is_err());
    }

    #[test]
    fn rejects_header_conditions_for_multiline_forms() {
        assert!(classify(true, 5, false, false, false, 0, 1).is_err());
    }

    #[test]
    fn rejects_marks_from_the_other_version() {
        assert!(classify(true, 6, false, false, false, 1, 1).is_err());
        assert!(classify(false, 1, true, false, false, 0, 1).is_err());
    }

    #[test]
    fn rejects_legacy_marks_not_declared_by_the_pattern() {
        assert!(classify(false, 0, false, false, false, 1, 0).is_err());
        assert!(classify(false, 1, false, false, false, 2, 1).is_err());
        assert!(classify(false, 2, false, false, false, 3, 1).is_err());
    }

    #[test]
    fn validates_each_legacy_mark_against_its_pattern() {
        assert!(super::validate_mark_captures(false, 1, 1, &[1]).is_ok());
        assert!(super::validate_mark_captures(false, 1, 1, &[2]).is_err());
        assert!(super::validate_mark_captures(false, 2, 0, &[1, 2]).is_ok());
        assert!(super::validate_mark_captures(true, 0, 0, &[1]).is_err());
    }

    #[test]
    fn follows_only_a_contiguous_unclosed_conditional_chain() {
        assert!(preceding_if(&[sibling("if", false), sibling("else-if", false)]).is_some());
        assert!(preceding_if(&[sibling("if", false), sibling("else", false)]).is_none());

        let mut unrelated = sibling("if", false);
        unrelated.element_class = Some("addon.OtherSection".to_owned());
        unrelated.metadata.clear();
        assert!(preceding_if(&[unrelated]).is_none());
    }

    #[test]
    fn recognizes_only_then_section_headers() {
        let then = SectionRawNode {
            raw_node_id: 2,
            kind: SectionRawNodeKind::Section,
            source: "then run:".to_owned(),
            span: span(),
        };
        assert!(is_then_header(Some(&then)));

        let mut effect = then.clone();
        effect.kind = SectionRawNodeKind::Simple;
        assert!(!is_then_header(Some(&effect)));
    }

    #[test]
    fn parse_if_uses_only_the_event_class_for_its_skript_generation() {
        assert_eq!(
            parse_if_event_class(false),
            "ch.njol.skript.events.bukkit.SkriptParseEvent"
        );
        assert_eq!(
            parse_if_event_class(true),
            "ch.njol.skript.lang.util.ContextlessEvent"
        );
    }

    #[test]
    fn conditional_delay_state_merges_independent_branches() {
        assert_eq!(
            merge_delay_states(ConditionalKind::If, "false", "true", None),
            ("true", "unknown")
        );
        assert_eq!(
            merge_delay_states(ConditionalKind::Else, "false", "true", Some("true")),
            ("true", "true")
        );
        assert_eq!(
            merge_delay_states(ConditionalKind::ElseIf, "false", "false", Some("true")),
            ("unknown", "unknown")
        );
        assert_eq!(
            merge_delay_states(ConditionalKind::If, "true", "true", None),
            ("true", "true")
        );
    }
}
