use super::{SourceColorSpan, SpanReport, render_source_colors};

#[derive(Clone, Copy)]
pub(super) struct SourceHighlight<'a> {
    input: &'a str,
    spans: &'a [SourceColorSpan],
    color: bool,
}

impl<'a> SourceHighlight<'a> {
    pub fn new(input: &'a str, spans: &'a [SourceColorSpan], color: bool) -> Self {
        Self {
            input,
            spans,
            color,
        }
    }

    pub fn quoted(self, source: &str, range: SpanReport) -> String {
        let quoted = format!("{source:?}");
        if !self.color || self.input.get(range.start..range.end) != Some(source) {
            return quoted;
        }
        // Project source byte ranges onto Debug's escaped representation, before adding ANSI.
        let escaped_offset = |offset| 1 + format!("{:?}", &source[..offset]).len() - 2;
        let spans = self
            .spans
            .iter()
            .filter_map(|colored| {
                self.input.get(colored.span.start..colored.span.end)?;
                let start = range.start.max(colored.span.start);
                let end = range.end.min(colored.span.end);
                (start < end).then(|| SourceColorSpan {
                    span: SpanReport {
                        start: escaped_offset(start - range.start),
                        end: escaped_offset(end - range.start),
                    },
                    ..*colored
                })
            })
            .collect::<Vec<_>>();
        render_source_colors(&quoted, &spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::SourceColor;

    #[test]
    fn quoted_snippets_rebase_clip_and_escape_without_touching_text() {
        for source in [
            "sin(1 + cos(2))",
            "あ\"\\\n\t\u{1b}[31m'",
            "e\u{301}\u{200d}🧑",
        ] {
            let input = format!("前 {source} 後");
            let range = SpanReport {
                start: "前 ".len(),
                end: "前 ".len() + source.len(),
            };
            let spans = [SourceColorSpan {
                span: SpanReport {
                    start: 0,
                    end: input.len(),
                },
                color: SourceColor::Function,
                depth: 0,
                order: 0,
            }];
            let colored = SourceHighlight::new(&input, &spans, true).quoted(source, range);
            assert_eq!(
                colored,
                format!(
                    "\"\x1b[{}m{}\x1b[0m\"",
                    SourceColor::Function.ansi_code(),
                    &format!("{source:?}")[1..format!("{source:?}").len() - 1]
                )
            );
            assert_eq!(
                SourceHighlight::new(&input, &spans, false).quoted(source, range),
                format!("{source:?}")
            );
            assert_eq!(
                SourceHighlight::new(&input, &spans, true)
                    .quoted(source, SpanReport { start: 0, end: 1 }),
                format!("{source:?}")
            );
        }
    }
}
