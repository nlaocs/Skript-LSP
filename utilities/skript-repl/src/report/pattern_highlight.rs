//! Experimental registration-pattern presentation; this is not Skript source syntax.
use super::{
    MAX_REPORT_PATTERN_DEPTH, PatternElement, SourceColor, SourceColorSpan, SpanReport,
    SpannedPatternElement,
};

pub(super) fn collect(source: &str, elements: &[SpannedPatternElement]) -> Vec<SourceColorSpan> {
    let mut spans = Vec::new();
    visit(source, elements, 0, &mut spans);
    spans
}

fn visit(
    source: &str,
    elements: &[SpannedPatternElement],
    depth: usize,
    spans: &mut Vec<SourceColorSpan>,
) {
    if depth >= MAX_REPORT_PATTERN_DEPTH {
        return;
    }
    for element in elements {
        let range = SpanReport {
            start: element.span.start,
            end: element.span.end,
        };
        match &element.value {
            PatternElement::TypeExpr(_) => push(source, range, SourceColor::TypeName, depth, spans),
            PatternElement::Regex(_) => push(source, range, SourceColor::Alias, depth, spans),
            PatternElement::ParseTag(_) | PatternElement::ParseMark(_) => {
                push(source, range, SourceColor::Function, depth, spans);
            }
            PatternElement::Group(children) | PatternElement::Option(children) => {
                let (open, close, color) = match &element.value {
                    PatternElement::Option(_) => ('[', ']', SourceColor::InterpolationDelimiter),
                    _ => ('(', ')', SourceColor::Effect),
                };
                if let Some(text) = source.get(range.start..range.end) {
                    if text.starts_with(open) && text.ends_with(close) {
                        push(
                            source,
                            SpanReport {
                                start: range.start,
                                end: range.start + 1,
                            },
                            color,
                            depth,
                            spans,
                        );
                        push(
                            source,
                            SpanReport {
                                start: range.end - 1,
                                end: range.end,
                            },
                            color,
                            depth,
                            spans,
                        );
                    }
                }
                visit(source, children, depth + 1, spans);
            }
            PatternElement::Choice(branches) => {
                // Only gaps between parsed branches contain grammar pipes. Escaped pipes
                // inside literal/regex/type nodes must keep their original role.
                for pair in branches.windows(2) {
                    if let (Some(left), Some(right)) = (pair[0].last(), pair[1].first()) {
                        if let Some(gap) = source.get(left.span.end..right.span.start) {
                            for (offset, ch) in gap.char_indices().filter(|(_, ch)| *ch == '|') {
                                let start = left.span.end + offset;
                                push(
                                    source,
                                    SpanReport {
                                        start,
                                        end: start + ch.len_utf8(),
                                    },
                                    SourceColor::Effect,
                                    depth,
                                    spans,
                                );
                            }
                        }
                    }
                }
                for branch in branches {
                    visit(source, branch, depth + 1, spans);
                }
            }
            PatternElement::Literal(_) | PatternElement::Empty => {}
        }
    }
}

fn push(
    source: &str,
    span: SpanReport,
    color: SourceColor,
    depth: usize,
    spans: &mut Vec<SourceColorSpan>,
) {
    if span.start < span.end && source.get(span.start..span.end).is_some() {
        spans.push(SourceColorSpan {
            span,
            color,
            depth,
            order: spans.len(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{PatternReport, parse};
    use syntax_pattern_parser::syntax::PluralRules;

    fn test_report(source: &str) -> PatternReport {
        let rules = PluralRules::from_json(include_str!(
            "../../tests/data/current-multi-addon-2.15.4/PluralRules.json"
        ))
        .unwrap();
        let colors = parse(source, &rules)
            .map(|parsed| collect(source, &parsed.elements))
            .unwrap_or_default();
        PatternReport {
            index: 0,
            source: source.to_owned(),
            elements: Vec::new(),
            colors,
        }
    }

    fn strip_sgr(text: &str) -> String {
        let mut plain = String::new();
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                assert_eq!(chars.next(), Some('['));
                assert!(chars.by_ref().any(|ch| ch == 'm'));
            } else {
                plain.push(ch);
            }
        }
        plain
    }

    #[test]
    fn registration_grammar_colors_preserve_original_spelling_and_escapes() {
        let source = r"(message|send [message[s]]) %objects% [to %audiences%]";
        let report = test_report(source);
        let colored = report.render_source(true);
        assert_eq!(strip_sgr(&colored), source);
        assert_eq!(report.render_source(false), source);
        assert!(colored.contains("\x1b[38;2;255;134;47m%objects%\x1b[0m"));
        assert!(colored.contains("message\x1b[38;2;88;196;221m|\x1b[0msend"));
        assert!(colored.contains("\x1b[38;2;176;176;176m[\x1b[0mto "));
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({"index":0,"source":source,"elements":[]})
        );

        for source in [
            r"日本語\|literal (a||b) [x[y]] %~dummyfixturepeople/objects@-1%",
            r"tag:foo <a|b> [|x]",
            r"literal\[x\] \%quoted\% ] )",
        ] {
            let report = test_report(source);
            assert_eq!(strip_sgr(&report.render_source(true)), source);
            assert_eq!(report.render_source(false), source);
            // Escaped literal delimiters never become grammar delimiters.
            assert!(!report.render_source(true).contains("literal\\\x1b["));
        }
    }

    #[test]
    fn incomplete_patterns_fall_back_to_exact_plain_source() {
        for source in [
            "[to %objects%",
            "(a|b",
            "%objects",
            "<regex",
            "%objects@bad%",
            "",
        ] {
            assert_eq!(test_report(source).render_source(true), source);
            assert_eq!(test_report(source).render_source(false), source);
        }
    }
}
