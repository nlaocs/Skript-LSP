use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, DiagnosticSeverity, DynamicMultiplicity, ExpressionExpectedType,
    ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload, MappedSpan, MetadataEntry,
    ParseRequest, ParseResult, ParseResultReference, ParseResultStatus, SourceOrigin, TextRange,
};

const EXPRESSION_PARSER_ID: &str = "host.expression";

pub(crate) enum Outcome {
    Requests(Vec<ParseRequest>),
    Candidate(ExpressionLeafCandidate, Vec<ParseResult>),
    Invalid(Diagnostic),
}

pub(crate) fn parse(payload: &ExpressionPayload, results: &[ParseResult]) -> Option<Outcome> {
    for end in payload.candidate_ends.iter().copied().rev() {
        let start = usize::try_from(payload.remaining.start).ok()?;
        let end = usize::try_from(end).ok()?;
        let text = payload.input.get(start..end)?;
        let Some(container) = Container::from_text(payload, text) else {
            continue;
        };
        let ranges = match embedded_ranges(container.body) {
            Ok(ranges) if ranges.is_empty() => continue,
            Ok(ranges) => ranges,
            Err(offset) => {
                return Some(Outcome::Invalid(Diagnostic {
                    code: "core.variable-string.unclosed-expression".to_owned(),
                    message: "an embedded Expression is missing its closing '%'".to_owned(),
                    severity: DiagnosticSeverity::Error,
                    span: subspan(
                        payload,
                        start.saturating_add(container.body_offset + offset),
                        start.saturating_add(container.body_offset + offset + 1),
                    ),
                    related: Vec::new(),
                }));
            }
        };
        let requests = ranges
            .iter()
            .enumerate()
            .map(|(index, range)| {
                let absolute_start = start + container.body_offset + range.start;
                let absolute_end = start + container.body_offset + range.end;
                ParseRequest {
                    request_id: index as u64,
                    parser_id: EXPRESSION_PARSER_ID.to_owned(),
                    input: container.body[range.clone()].to_owned(),
                    expected_types: vec![ExpressionExpectedType {
                        class_name: "java.lang.Object".to_owned(),
                        plural: true,
                    }],
                    span: subspan(payload, absolute_start, absolute_end),
                    options: vec![MetadataEntry {
                        key: "container".to_owned(),
                        value: container.kind.to_owned(),
                    }],
                }
            })
            .collect::<Vec<_>>();
        if results.is_empty() {
            return Some(Outcome::Requests(requests));
        }
        if results.len() != requests.len()
            || requests.iter().any(|request| {
                !results.iter().any(|result| {
                    result.request_id == request.request_id
                        && result.parser_id == request.parser_id
                        && matches!(result.status, ParseResultStatus::Success)
                })
            })
        {
            return Some(Outcome::Invalid(
                results
                    .iter()
                    .flat_map(|result| result.diagnostics.iter())
                    .next()
                    .cloned()
                    .unwrap_or_else(|| Diagnostic {
                        code: "core.variable-string.invalid-expression".to_owned(),
                        message: "an embedded Expression could not be parsed".to_owned(),
                        severity: DiagnosticSeverity::Error,
                        span: payload.span.clone(),
                        related: Vec::new(),
                    }),
            ));
        }
        let mut leaf = candidate(
            container.parser_id,
            container.leaf_kind,
            payload.remaining.start,
            end as u64,
            container.return_type(payload),
            container.multiplicity(text),
        );
        let expression_count = requests.len().to_string();
        leaf.metadata
            .push(metadata("embedded-expression-count", &expression_count));
        leaf.children = results
            .iter()
            .flat_map(|result| {
                result.roots.iter().map(|root_id| ParseResultReference {
                    host_token: result.host_token,
                    root_id: *root_id,
                })
            })
            .collect();
        return Some(Outcome::Candidate(leaf, results.to_vec()));
    }
    None
}

struct Container<'a> {
    kind: &'static str,
    parser_id: &'static str,
    leaf_kind: ExpressionLeafKind,
    body: &'a str,
    body_offset: usize,
}

impl<'a> Container<'a> {
    fn from_text(payload: &ExpressionPayload, text: &'a str) -> Option<Self> {
        if payload.allow_literals && text.len() >= 2 && text.starts_with('"') && text.ends_with('"')
        {
            return Some(Self {
                kind: "string",
                parser_id: "core.literal.variable-string",
                leaf_kind: ExpressionLeafKind::Literal,
                body: &text[1..text.len() - 1],
                body_offset: 1,
            });
        }
        if payload.allow_expressions
            && text.len() >= 3
            && text.starts_with('{')
            && text.ends_with('}')
        {
            return Some(Self {
                kind: "variable-name",
                parser_id: "core.variable",
                leaf_kind: ExpressionLeafKind::Variable,
                body: &text[1..text.len() - 1],
                body_offset: 1,
            });
        }
        None
    }

    fn return_type<'b>(&self, payload: &'b ExpressionPayload) -> &'b str {
        if self.kind == "string" {
            "java.lang.String"
        } else {
            payload
                .expected_types
                .first()
                .map_or("java.lang.Object", |expected| expected.class_name.as_str())
        }
    }

    fn multiplicity(&self, text: &str) -> DynamicMultiplicity {
        if self.kind == "variable-name" && text[1..text.len() - 1].trim_end().ends_with("::*") {
            DynamicMultiplicity::Multiple
        } else {
            DynamicMultiplicity::Single
        }
    }
}

fn embedded_ranges(input: &str) -> Result<Vec<std::ops::Range<usize>>, usize> {
    let bytes = input.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'%' {
            cursor += 1;
            continue;
        }
        if bytes.get(cursor + 1) == Some(&b'%') {
            cursor += 2;
            continue;
        }
        let opening = cursor;
        cursor += 1;
        let start = cursor;
        let mut braces = 0usize;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'{' => braces = braces.saturating_add(1),
                b'}' if braces > 0 => braces -= 1,
                b'%' if braces == 0 => break,
                _ => {}
            }
            cursor += 1;
        }
        if cursor == bytes.len() {
            return Err(opening);
        }
        ranges.push(start..cursor);
        cursor += 1;
    }
    Ok(ranges)
}

fn subspan(payload: &ExpressionPayload, absolute_start: usize, absolute_end: usize) -> MappedSpan {
    let remaining_start = payload.remaining.start;
    let relative_start = (absolute_start as u64).saturating_sub(remaining_start);
    let relative_end = (absolute_end as u64).saturating_sub(remaining_start);
    let input_len = payload
        .span
        .virtual_range
        .end
        .saturating_sub(payload.span.virtual_range.start);
    MappedSpan {
        virtual_range: TextRange {
            start: payload
                .span
                .virtual_range
                .start
                .saturating_add(relative_start),
            end: payload
                .span
                .virtual_range
                .start
                .saturating_add(relative_end),
        },
        origins: payload
            .span
            .origins
            .iter()
            .map(|origin| SourceOrigin {
                original_range: if origin
                    .original_range
                    .end
                    .saturating_sub(origin.original_range.start)
                    >= input_len
                {
                    TextRange {
                        start: origin.original_range.start.saturating_add(relative_start),
                        end: origin.original_range.start.saturating_add(relative_end),
                    }
                } else {
                    origin.original_range
                },
                kind: origin.kind,
                expansion: origin.expansion,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::embedded_ranges;

    #[test]
    fn finds_expressions_and_skips_escaped_percent_signs() {
        assert_eq!(
            embedded_ranges("100%%, %size of all players%").unwrap(),
            vec![8..27]
        );
    }

    #[test]
    fn ignores_percent_signs_inside_nested_variables() {
        assert_eq!(
            embedded_ranges("%{data::%event-player%}%").unwrap(),
            vec![1..23]
        );
    }

    #[test]
    fn rejects_an_unclosed_expression() {
        assert_eq!(embedded_ranges("hello %player"), Err(6));
    }
}
