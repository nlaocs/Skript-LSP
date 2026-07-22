use syntax_pattern_parser::syntax::{
    self, ParseResult, PatternElement, PluralRules, Span, SpannedPatternElement,
};

#[derive(Debug)]
pub struct SpanFailure {
    pub message: String,
    pub span: Span,
}

pub fn validate_parse_invariants(pattern: &str, plural_rules: &PluralRules) -> Result<(), String> {
    let first = syntax::parse(pattern, plural_rules);
    let second = syntax::parse(pattern, plural_rules);

    if first != second {
        return Err("the same input produced different parse results".to_string());
    }

    match first {
        Ok(result) => validate_parse_result(pattern, &result)
            .map_err(|failure| format!("{} at {:?}", failure.message, failure.span)),
        Err(error) => validate_span(pattern, error.span, "parse error"),
    }
}

pub fn validate_parse_result(pattern: &str, result: &ParseResult) -> Result<(), SpanFailure> {
    validate_elements(pattern, &result.elements, None, "root")?;

    for (index, warning) in result.warnings.iter().enumerate() {
        if !warning.span.is_valid_for(pattern) {
            return Err(SpanFailure {
                message: format!("warning[{index}] has an invalid UTF-8 source span"),
                span: warning.span,
            });
        }
    }

    Ok(())
}

fn validate_elements(
    pattern: &str,
    elements: &[SpannedPatternElement],
    parent: Option<Span>,
    path: &str,
) -> Result<(), SpanFailure> {
    for (index, element) in elements.iter().enumerate() {
        let element_path = format!("{path}[{index}].{}", element_kind(&element.value));

        if !element.span.is_valid_for(pattern) {
            return Err(SpanFailure {
                message: format!("{element_path} has an invalid UTF-8 source span"),
                span: element.span,
            });
        }

        if let Some(parent) = parent
            && (element.span.start < parent.start || element.span.end > parent.end)
        {
            return Err(SpanFailure {
                message: format!(
                    "{element_path} is outside parent span {}..{}",
                    parent.start, parent.end
                ),
                span: element.span,
            });
        }

        match &element.value {
            PatternElement::Choice(branches) => {
                for (branch_index, branch) in branches.iter().enumerate() {
                    validate_elements(
                        pattern,
                        branch,
                        Some(element.span),
                        &format!("{element_path}.branch[{branch_index}]"),
                    )?;
                }
            }
            PatternElement::Group(children) | PatternElement::Option(children) => {
                validate_elements(pattern, children, Some(element.span), &element_path)?;
            }
            PatternElement::Empty if element.span.start != element.span.end => {
                return Err(SpanFailure {
                    message: format!("{element_path} must have a zero-width span"),
                    span: element.span,
                });
            }
            _ => {}
        }
    }

    Ok(())
}

fn validate_span(pattern: &str, span: Span, label: &str) -> Result<(), String> {
    if span.is_valid_for(pattern) {
        Ok(())
    } else {
        Err(format!(
            "{label} has invalid span {}..{} for {} UTF-8 bytes",
            span.start,
            span.end,
            pattern.len()
        ))
    }
}

fn element_kind(element: &PatternElement) -> &'static str {
    match element {
        PatternElement::Literal(_) => "literal",
        PatternElement::Choice(_) => "choice",
        PatternElement::Group(_) => "group",
        PatternElement::Option(_) => "option",
        PatternElement::Regex(_) => "regex",
        PatternElement::TypeExpr(_) => "typeExpression",
        PatternElement::ParseTag(_) => "parseTag",
        PatternElement::ParseMark(_) => "parseMark",
        PatternElement::Empty => "empty",
    }
}
