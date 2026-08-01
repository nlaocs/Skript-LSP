//! Grammar, AST, spans, and diagnostics for Skript registration patterns.
//!
//! The parser consumes one registered pattern, preserves UTF-8 byte ranges, and
//! normalizes type alternatives with the supplied server-specific plural rules.

use crate::plural::PluralRules;

macro_rules! consume_until {
    ($chars:expr, $end:expr) => {{
        use std::cmp::Ordering;
        while let Some(&(j, _)) = $chars.peek() {
            match j.cmp(&$end) {
                Ordering::Less => {
                    // j < end
                    $chars.next();
                }
                Ordering::Equal => {
                    // j == end
                    $chars.next(); // consume end char
                    break;
                }
                Ordering::Greater => {
                    // shouldn't happen
                    break;
                }
            }
        }
    }};
}

/// A half-open UTF-8 byte range in the original syntax pattern.
///
/// Both offsets are guaranteed to be character boundaries for parser output.
/// Offsets count bytes rather than Unicode scalar values, matching Rust string
/// slicing and the Language Server Protocol's source-mapping layer.
///
/// # Examples
///
/// ~~~
/// use syntax_pattern_parser::syntax::Span;
///
/// let source = "send 日本語";
/// let japanese = Span::new(5, source.len());
///
/// assert!(japanese.is_valid_for(source));
/// assert_eq!(japanese.slice(source), Some("日本語"));
///
/// // Byte 6 lies inside the three-byte encoding of 日.
/// assert!(!Span::new(5, 6).is_valid_for(source));
/// ~~~
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}

impl Span {
    /// Creates a half-open `start..end` range.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns whether the range is ordered, in bounds, and on UTF-8 boundaries.
    pub fn is_valid_for(self, input: &str) -> bool {
        self.start <= self.end
            && self.end <= input.len()
            && input.is_char_boundary(self.start)
            && input.is_char_boundary(self.end)
    }

    /// Returns the covered substring, or `None` when the range is invalid.
    pub fn slice(self, input: &str) -> Option<&str> {
        input.get(self.start..self.end)
    }
}

/// A parsed value paired with the source range that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    /// Parsed value.
    pub value: T,
    /// Range that produced `value`, including delimiters when applicable.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Associates a parsed value with its source range.
    pub const fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }

    /// Transforms the value while preserving its source range.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned::new(f(self.value), self.span)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// One node in a parsed registration-pattern AST.
pub enum PatternElement {
    /// Literal text matched according to Skript's whitespace rules.
    Literal(String),
    /// `|`-separated branches; each branch is a sequence of elements.
    Choice(Vec<Vec<SpannedPatternElement>>),
    /// Parenthesized sequence such as `(group)`.
    Group(Vec<SpannedPatternElement>),
    /// Optional bracketed sequence such as `[text]`.
    Option(Vec<SpannedPatternElement>),
    /// Regex body without its `<` and `>` delimiters.
    Regex(String),
    /// Typed expression placeholder such as `%strings%`.
    TypeExpr(PatternTypeExpr),
    /// Colon parse tag attached to the following element.
    ParseTag(String),
    /// Numeric parse mark attached with `¦`.
    ParseMark(i32),
    /// Explicitly empty choice branch, for example the second branch of `a|`.
    Empty,
} // todo display実装

/// A syntax pattern AST element with its range in the original pattern.
pub type SpannedPatternElement = Spanned<PatternElement>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Parsed contents of a `%...%` expression placeholder.
pub struct PatternTypeExpr {
    /// Slash-separated accepted types, normalized to singular names.
    pub alternatives: Vec<PatternTypeAlternative>,
    /// Whether Skript's `-` flag permits a missing expression.
    pub nullable: bool,
    /// Whether literal expressions are permitted (`~` disables them).
    pub allow_literals: bool,
    /// Whether non-literal expressions are permitted (`*` disables them).
    pub allow_expressions: bool,
    /// Skript time-state suffix; zero means the current state.
    pub time: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// One normalized type name inside a type expression.
pub struct PatternTypeAlternative {
    /// Singular type code name.
    pub name: String,
    /// Whether the source spelling was plural.
    pub plural: bool,
}

/// Display adapter that reconstructs a type expression with active plural rules.
pub struct PatternTypeExprDisplay<'a> {
    type_expr: &'a PatternTypeExpr,
    plural_rules: &'a PluralRules,
}

impl PatternTypeExpr {
    /// Returns a display adapter using `plural_rules` for plural alternatives.
    pub fn display_with<'a>(&'a self, plural_rules: &'a PluralRules) -> PatternTypeExprDisplay<'a> {
        PatternTypeExprDisplay {
            type_expr: self,
            plural_rules,
        }
    }
}

impl std::fmt::Display for PatternTypeExprDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let type_expr = self.type_expr;
        if type_expr.nullable {
            write!(f, "-")?;
        }
        if !type_expr.allow_literals {
            write!(f, "~")?;
        } else if !type_expr.allow_expressions {
            write!(f, "*")?;
        }
        for (index, alternative) in type_expr.alternatives.iter().enumerate() {
            if index != 0 {
                write!(f, "/")?;
            }
            if alternative.plural {
                write!(f, "{}", self.plural_rules.to_plural(&alternative.name))?;
            } else {
                write!(f, "{}", alternative.name)?;
            }
        }
        if type_expr.time != 0 {
            write!(f, "@{}", type_expr.time)?;
        }
        Ok(())
    }
}

fn parse_pattern_type_expr(
    input: &str,
    input_offset: usize,
    plural_rules: &PluralRules,
) -> Result<PatternTypeExpr, ParseError> {
    let mut body = input;
    let mut nullable = false;
    let mut allow_literals = true;
    let mut allow_expressions = true;

    loop {
        match body.as_bytes().first() {
            Some(b'-') => nullable = true,
            Some(b'*') => allow_expressions = false,
            Some(b'~') => allow_literals = false,
            _ => break,
        }
        body = &body[1..];
    }

    let body_offset = input.len() - body.len();
    let (alternatives, time) = if let Some(time_start) = body.find('@') {
        let time = body[time_start + 1..].parse::<i32>().map_err(|_| {
            parse_error(
                ParseErrorKind::IncorrectTimeState,
                Span::new(
                    input_offset + body_offset + time_start + 1,
                    input_offset + input.len(),
                ),
            )
        })?;
        (&body[..time_start], time)
    } else {
        (body, 0)
    };

    Ok(PatternTypeExpr {
        alternatives: alternatives
            .split('/')
            .map(|word| {
                let (name, plural) = plural_rules.to_singular(word);
                PatternTypeAlternative { name, plural }
            })
            .collect(),
        nullable,
        allow_literals,
        allow_expressions,
        time,
    })
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq, Hash)]
/// Fatal registration-pattern parse failure.
pub enum ParseErrorKind {
    #[error(
        "Missing closing group bracket ')'. Escape the '(' if you want to match a literal bracket: '\\('"
    )]
    /// A group reached EOF without `)`.
    UnclosedParenthesis,
    #[error(
        "Missing closing optional bracket ']'. Escape the '[' if you want to match a literal bracket: '\\['"
    )]
    /// An optional sequence reached EOF without `]`.
    UnclosedBracket,
    #[error(
        "Missing closing type delimiter '%'. Escape the '%' if you want to match a literal percent sign: '\\%'"
    )]
    /// A type expression reached EOF without its second `%`.
    UnclosedTypeDelimiter,
    #[error(
        "Missing closing regex bracket '>'. Escape the '<' if you want to match a literal bracket: '\\<'"
    )]
    /// A regex reached EOF without `>`.
    UnclosedRegexDelimiter,
    #[error("Incorrect time state in type expression. It must be a 32-bit signed integer.")]
    /// A type-expression `@` suffix was not a signed 32-bit integer.
    IncorrectTimeState,
    #[error("Invalid parse mark. Text before '¦' must be a 32-bit signed integer.")]
    /// Text before `¦` was not a signed 32-bit integer.
    InvalidParseMark,
}

/// The role of a secondary source range attached to a parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelatedSpanKind {
    /// The delimiter that opened a construct which was not closed.
    OpeningDelimiter,
}

/// A secondary source range that provides context for a parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelatedSpan {
    /// Why the related location matters.
    pub kind: RelatedSpanKind,
    /// Related range in the original registration pattern.
    pub span: Span,
}

impl RelatedSpan {
    /// Creates related source information with a typed role.
    pub const fn new(kind: RelatedSpanKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// A syntax pattern error with a primary range and optional related ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParseError {
    /// Stable error classification.
    pub kind: ParseErrorKind,
    /// Primary range, often a zero-width EOF location for unclosed input.
    pub span: Span,
    /// Additional locations such as the opening delimiter of an unclosed construct.
    pub related_spans: Vec<RelatedSpan>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte range {:?}", self.kind, self.span)?;
        if !self.related_spans.is_empty() {
            write!(f, "; related spans: {:?}", self.related_spans)?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    fn new(kind: ParseErrorKind, span: Span) -> Self {
        Self {
            kind,
            span,
            related_spans: Vec::new(),
        }
    }

    fn with_related_span(mut self, kind: RelatedSpanKind, span: Span) -> Self {
        self.related_spans.push(RelatedSpan::new(kind, span));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display)]
/// Non-fatal compatibility concern preserved alongside a successful AST.
pub enum ParseWarningKind {
    #[display(
        "Label not supported. However, it may be supported in the future (this has no effect on end users)."
    )]
    /// A label-like form was retained but has no parser meaning yet.
    LabelNotSupported,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Non-fatal parse warning with its source range.
pub struct ParseWarning {
    /// Stable warning classification.
    pub kind: ParseWarningKind,
    /// Range that triggered the warning.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Successfully parsed top-level sequence and non-fatal warnings.
pub struct ParseResult {
    /// Top-level AST elements in registration order.
    pub elements: Vec<SpannedPatternElement>,
    /// Compatibility warnings discovered while parsing.
    pub warnings: Vec<ParseWarning>,
}

/// Parses one complete Skript registration pattern using server-specific plural rules.
///
/// The returned tree retains every element's UTF-8 byte span. Type names are
/// normalized with the exact plural rules captured from the target server, so
/// callers should not substitute a hardcoded English singularization table.
///
/// # Examples
///
/// This example inspects a typed placeholder and a grouped choice:
///
/// ~~~
/// use syntax_pattern_parser::syntax::{
///     parse, PatternElement, PluralRules,
/// };
///
/// # fn rules() -> PluralRules {
/// #     PluralRules::from_json(r#"{
/// #         "algorithm": "singular-aware",
/// #         "pluralOverrideSupported": false,
/// #         "rules": [{
/// #             "ruleOrder": 0,
/// #             "singular": "",
/// #             "plural": "s",
/// #             "completeWord": false,
/// #             "origin": "built-in",
/// #             "addon": { "name": "Skript", "version": "example" }
/// #         }]
/// #     }"#).expect("example plural rules are valid")
/// # }
/// let source = "send %strings% to (console|player)";
/// let parsed = parse(source, &rules())?;
///
/// let PatternElement::TypeExpr(expression) = &parsed.elements[1].value else {
///     panic!("the second element must be a type expression");
/// };
/// assert_eq!(expression.alternatives[0].name, "string");
/// assert!(expression.alternatives[0].plural);
/// assert_eq!(parsed.elements[1].span.slice(source), Some("%strings%"));
///
/// let PatternElement::Group(group) = &parsed.elements[3].value else {
///     panic!("the fourth element must be a group");
/// };
/// assert!(matches!(group[0].value, PatternElement::Choice(_)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ~~~
///
/// Unclosed constructs point at EOF and retain the opening delimiter as a
/// related span, allowing an editor to render both locations:
///
/// ~~~
/// use syntax_pattern_parser::syntax::{
///     parse, ParseErrorKind, PluralRules, RelatedSpanKind, Span,
/// };
///
/// # let rules = PluralRules::from_json(r#"{
/// #     "algorithm": "singular-aware",
/// #     "pluralOverrideSupported": false,
/// #     "rules": [{
/// #         "ruleOrder": 0, "singular": "", "plural": "s",
/// #         "completeWord": false, "origin": "built-in",
/// #         "addon": { "name": "Skript", "version": "example" }
/// #     }]
/// # }"#).unwrap();
/// let error = parse("[(group]", &rules).unwrap_err();
///
/// assert_eq!(error.kind, ParseErrorKind::UnclosedParenthesis);
/// assert_eq!(error.span, Span::new(8, 8));
/// assert_eq!(error.related_spans[0].kind, RelatedSpanKind::OpeningDelimiter);
/// assert_eq!(error.related_spans[0].span, Span::new(1, 2));
/// ~~~
///
/// # Errors
///
/// Returns a ranged [`ParseError`] for unclosed delimiters, invalid time states,
/// or invalid parse marks.
pub fn parse(input: &str, plural_rules: &PluralRules) -> Result<ParseResult, ParseError> {
    let mut chars = input.char_indices().peekable();
    parse_choice(&mut chars, Scope::Global, input, plural_rules)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Scope {
    Global,
    Group { opening_span: Span },
    Option { opening_span: Span },
}

impl Scope {
    fn is_group(self) -> bool {
        matches!(self, Self::Group { .. })
    }

    fn is_option(self) -> bool {
        matches!(self, Self::Option { .. })
    }
}

fn current_offset<I: Iterator<Item = (usize, char)>>(
    chars: &mut std::iter::Peekable<I>,
    input_len: usize,
) -> usize {
    chars.peek().map_or(input_len, |(offset, _)| *offset)
}

fn parse_error(kind: ParseErrorKind, span: Span) -> ParseError {
    ParseError::new(kind, span)
}

fn push_literal(
    elements: &mut Vec<SpannedPatternElement>,
    buffer: &mut String,
    buffer_start: &mut Option<usize>,
    end: usize,
) {
    if buffer.is_empty() {
        *buffer_start = None;
        return;
    }

    let start = buffer_start.take().unwrap_or(end);
    elements.push(Spanned::new(
        PatternElement::Literal(std::mem::take(buffer)),
        Span::new(start, end),
    ));
}

fn parse_sequence<I: Iterator<Item = (usize, char)> + Clone>(
    chars: &mut std::iter::Peekable<I>,
    scope: Scope,
    raw_pattern: &str,
    plural_rules: &PluralRules,
) -> Result<ParseResult, ParseError> {
    let mut elements = Vec::new();
    let mut buffer = String::new();
    let mut buffer_start = None;
    let mut warnings = Vec::new();

    while let Some(&(i, ch)) = chars.peek() {
        match ch {
            '(' => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                chars.next();

                let group = parse_choice(
                    chars,
                    Scope::Group {
                        opening_span: Span::new(i, i + ch.len_utf8()),
                    },
                    raw_pattern,
                    plural_rules,
                )?;
                warnings.extend(group.warnings);
                let end = current_offset(chars, raw_pattern.len());
                elements.push(Spanned::new(
                    PatternElement::Group(group.elements),
                    Span::new(i, end),
                ));
            }
            ')' if scope.is_group() => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                break;
            }
            '[' => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                chars.next();

                let option = parse_choice(
                    chars,
                    Scope::Option {
                        opening_span: Span::new(i, i + ch.len_utf8()),
                    },
                    raw_pattern,
                    plural_rules,
                )?;
                warnings.extend(option.warnings);
                let end = current_offset(chars, raw_pattern.len());
                elements.push(Spanned::new(
                    PatternElement::Option(option.elements),
                    Span::new(i, end),
                ));
            }
            ']' if scope.is_option() => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                break;
            }
            '<' => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                chars.next();

                let content_start = i + ch.len_utf8();
                if let Some(relative_end) = raw_pattern[content_start..].find('>') {
                    let content_end = content_start + relative_end;
                    let regex = &raw_pattern[content_start..content_end];

                    consume_until!(chars, content_end);
                    elements.push(Spanned::new(
                        PatternElement::Regex(regex.to_string()),
                        Span::new(i, content_end + '>'.len_utf8()),
                    ));
                } else {
                    let end = raw_pattern.len();
                    return Err(parse_error(
                        ParseErrorKind::UnclosedRegexDelimiter,
                        Span::new(end, end),
                    )
                    .with_related_span(
                        RelatedSpanKind::OpeningDelimiter,
                        Span::new(i, content_start),
                    ));
                }
            }
            // Skript's PatternCompiler treats unmatched closing delimiters as literal text.
            ')' | ']' | '>' => {
                buffer_start.get_or_insert(i);
                buffer.push(ch);
                chars.next();
            }
            '%' => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                chars.next();

                let content_start = i + ch.len_utf8();
                if let Some(relative_end) = raw_pattern[content_start..].find('%') {
                    let content_end = content_start + relative_end;
                    let type_expr = parse_pattern_type_expr(
                        &raw_pattern[content_start..content_end],
                        content_start,
                        plural_rules,
                    )?;

                    consume_until!(chars, content_end);
                    elements.push(Spanned::new(
                        PatternElement::TypeExpr(type_expr),
                        Span::new(i, content_end + '%'.len_utf8()),
                    ));
                } else {
                    let end = raw_pattern.len();
                    return Err(parse_error(
                        ParseErrorKind::UnclosedTypeDelimiter,
                        Span::new(end, end),
                    )
                    .with_related_span(
                        RelatedSpanKind::OpeningDelimiter,
                        Span::new(i, content_start),
                    ));
                }
            }
            '|' => {
                push_literal(&mut elements, &mut buffer, &mut buffer_start, i);
                break;
            }
            '\\' => {
                buffer_start.get_or_insert(i);
                chars.next();
                if let Some(&(_, escaped)) = chars.peek() {
                    buffer.push(escaped);
                    chars.next();
                } else {
                    buffer.push('\\');
                }
            }
            ':' => {
                let start = buffer_start.take().unwrap_or(i);
                elements.push(Spanned::new(
                    PatternElement::ParseTag(std::mem::take(&mut buffer)),
                    Span::new(start, i + ch.len_utf8()),
                ));
                chars.next();
            }
            '¦' => {
                let start = buffer_start.take().unwrap_or(i);
                let mark_text = std::mem::take(&mut buffer);
                let mark = mark_text.parse::<i32>().map_err(|_| {
                    parse_error(ParseErrorKind::InvalidParseMark, Span::new(start, i))
                })?;
                elements.push(Spanned::new(
                    PatternElement::ParseMark(mark),
                    Span::new(start, i + ch.len_utf8()),
                ));
                chars.next();
            }
            _ => {
                buffer_start.get_or_insert(i);
                buffer.push(ch);
                chars.next();
            }
        }
    }

    let end = current_offset(chars, raw_pattern.len());
    push_literal(&mut elements, &mut buffer, &mut buffer_start, end);

    Ok(ParseResult { elements, warnings })
}

fn parse_choice<I: Iterator<Item = (usize, char)> + Clone>(
    chars: &mut std::iter::Peekable<I>,
    scope: Scope,
    raw_pattern: &str,
    plural_rules: &PluralRules,
) -> Result<ParseResult, ParseError> {
    let choice_start = current_offset(chars, raw_pattern.len());
    let mut branches: Vec<Vec<SpannedPatternElement>> = Vec::new();
    let mut closed = false;
    let mut warnings = Vec::new();

    let choice_end = loop {
        let branch_start = current_offset(chars, raw_pattern.len());
        let sequence = parse_sequence(chars, scope, raw_pattern, plural_rules)?;
        warnings.extend(sequence.warnings);
        let branch_end = current_offset(chars, raw_pattern.len());

        if sequence.elements.is_empty() {
            branches.push(vec![Spanned::new(
                PatternElement::Empty,
                Span::new(branch_start, branch_start),
            )]);
        } else {
            branches.push(sequence.elements);
        }

        match chars.peek() {
            Some(&(_, '|')) => {
                chars.next();
            }
            Some(&(_, ')')) if scope.is_group() => {
                chars.next();
                closed = true;
                break branch_end;
            }
            Some(&(_, ']')) if scope.is_option() => {
                chars.next();
                closed = true;
                break branch_end;
            }
            None => break branch_end,
            _ => break branch_end,
        }
    };

    match scope {
        Scope::Group { opening_span } if !closed => {
            let end = raw_pattern.len();
            return Err(
                parse_error(ParseErrorKind::UnclosedParenthesis, Span::new(end, end))
                    .with_related_span(RelatedSpanKind::OpeningDelimiter, opening_span),
            );
        }
        Scope::Option { opening_span } if !closed => {
            let end = raw_pattern.len();
            return Err(
                parse_error(ParseErrorKind::UnclosedBracket, Span::new(end, end))
                    .with_related_span(RelatedSpanKind::OpeningDelimiter, opening_span),
            );
        }
        _ => {}
    }

    if branches.len() == 1 {
        Ok(ParseResult {
            elements: branches.into_iter().next().unwrap(),
            warnings,
        })
    } else {
        Ok(ParseResult {
            elements: vec![Spanned::new(
                PatternElement::Choice(branches),
                Span::new(choice_start, choice_end),
            )],
            warnings,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    static TEST_PLURAL_RULES: std::sync::LazyLock<PluralRules> = std::sync::LazyLock::new(|| {
        PluralRules::from_json(include_str!("../tests/data/PluralRules-2.15.4.json"))
            .expect("generated PluralRules-2.15.4.json fixture must be valid")
    });

    fn parse(input: &str) -> Result<ParseResult, ParseError> {
        super::parse(input, &TEST_PLURAL_RULES)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SemanticElement {
        Literal(String),
        Choice(Vec<Vec<SemanticElement>>),
        Group(Vec<SemanticElement>),
        Option(Vec<SemanticElement>),
        Regex(String),
        TypeExpr(PatternTypeExpr),
        ParseTag(String),
        ParseMark(i32),
        Empty,
    }

    fn semantic_element(element: &SpannedPatternElement) -> SemanticElement {
        match &element.value {
            PatternElement::Literal(value) => SemanticElement::Literal(value.clone()),
            PatternElement::Choice(branches) => SemanticElement::Choice(
                branches
                    .iter()
                    .map(|branch| semantic_elements(branch))
                    .collect(),
            ),
            PatternElement::Group(elements) => SemanticElement::Group(semantic_elements(elements)),
            PatternElement::Option(elements) => {
                SemanticElement::Option(semantic_elements(elements))
            }
            PatternElement::Regex(value) => SemanticElement::Regex(value.clone()),
            PatternElement::TypeExpr(value) => SemanticElement::TypeExpr(value.clone()),
            PatternElement::ParseTag(value) => SemanticElement::ParseTag(value.clone()),
            PatternElement::ParseMark(value) => SemanticElement::ParseMark(*value),
            PatternElement::Empty => SemanticElement::Empty,
        }
    }

    fn semantic_elements(elements: &[SpannedPatternElement]) -> Vec<SemanticElement> {
        elements.iter().map(semantic_element).collect()
    }

    fn type_element(
        alternatives: &[(&str, bool)],
        nullable: bool,
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
    ) -> SemanticElement {
        SemanticElement::TypeExpr(PatternTypeExpr {
            alternatives: alternatives
                .iter()
                .map(|(name, plural)| PatternTypeAlternative {
                    name: (*name).to_string(),
                    plural: *plural,
                })
                .collect(),
            nullable,
            allow_literals,
            allow_expressions,
            time,
        })
    }

    fn assert_semantics(input: &str, expected: Vec<SemanticElement>) {
        let result = parse(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));
        assert!(result.warnings.is_empty());
        assert_eq!(semantic_elements(&result.elements), expected);
    }

    fn assert_error(input: &str, kind: ParseErrorKind, span: Span) {
        let error = parse(input).expect_err("pattern must fail");
        assert_eq!(error.kind, kind, "{input:?}");
        assert_eq!(error.span, span, "{input:?}");
        assert!(error.span.is_valid_for(input), "{input:?}: {error:?}");
        for related in &error.related_spans {
            assert!(related.span.is_valid_for(input), "{input:?}: {error:?}");
        }
    }

    fn assert_valid_spans(input: &str, elements: &[SpannedPatternElement], parent: Option<Span>) {
        for element in elements {
            assert!(
                element.span.is_valid_for(input),
                "{:?} is invalid for {input:?}",
                element.span
            );
            if let Some(parent) = parent {
                assert!(
                    parent.start <= element.span.start && element.span.end <= parent.end,
                    "{:?} is outside parent {parent:?}",
                    element.span
                );
            }

            match &element.value {
                PatternElement::Choice(branches) => {
                    for branch in branches {
                        assert_valid_spans(input, branch, Some(element.span));
                    }
                }
                PatternElement::Group(children) | PatternElement::Option(children) => {
                    assert_valid_spans(input, children, Some(element.span));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn parses_basic_elements_and_choices() {
        use SemanticElement::*;

        assert_semantics("literal", vec![Literal("literal".into())]);
        assert_semantics(
            "(choice1|choice2)",
            vec![Group(vec![Choice(vec![
                vec![Literal("choice1".into())],
                vec![Literal("choice2".into())],
            ])])],
        );
        assert_semantics(
            "[foo|bar]",
            vec![Option(vec![Choice(vec![
                vec![Literal("foo".into())],
                vec![Literal("bar".into())],
            ])])],
        );
        assert_semantics("<[0-9]+>", vec![Regex("[0-9]+".into())]);
        assert_semantics(
            "folder|dir|box",
            vec![Choice(vec![
                vec![Literal("folder".into())],
                vec![Literal("dir".into())],
                vec![Literal("box".into())],
            ])],
        );
        assert_semantics(
            r"<.+> \|\| <.+>",
            vec![
                Regex(".+".into()),
                Literal(" || ".into()),
                Regex(".+".into()),
            ],
        );
        assert_semantics(
            "active[ |-](group|model)[s]",
            vec![
                Literal("active".into()),
                Option(vec![Choice(vec![
                    vec![Literal(" ".into())],
                    vec![Literal("-".into())],
                ])]),
                Group(vec![Choice(vec![
                    vec![Literal("group".into())],
                    vec![Literal("model".into())],
                ])]),
                Option(vec![Literal("s".into())]),
            ],
        );
    }

    #[test]
    fn parses_type_expression_modifiers_and_plural_rules() {
        for (input, expected) in [
            (
                "%string%",
                type_element(&[("string", false)], false, true, true, 0),
            ),
            (
                "%*string%",
                type_element(&[("string", false)], false, true, false, 0),
            ),
            (
                "%~string%",
                type_element(&[("string", false)], false, false, true, 0),
            ),
            (
                "%-string%",
                type_element(&[("string", false)], true, true, true, 0),
            ),
        ] {
            assert_semantics(input, vec![expected]);
        }

        assert_semantics(
            "%-*~strings/locations@12%",
            vec![type_element(
                &[("string", true), ("location", true)],
                true,
                false,
                false,
                12,
            )],
        );
        assert_semantics(
            "%string/*numbers/-texts%",
            vec![type_element(
                &[("string", false), ("*number", true), ("-text", true)],
                false,
                true,
                true,
                0,
            )],
        );
        assert_semantics(
            "%children/aliases/axes/sheep/sheeps/dummyfixturepeople%",
            vec![type_element(
                &[
                    ("child", true),
                    ("alias", true),
                    ("axe", true),
                    ("sheep", false),
                    ("sheep", true),
                    ("dummyfixtureperson", true),
                ],
                false,
                true,
                true,
                0,
            )],
        );

        for (input, time) in [
            ("%string@2147483647%", i32::MAX),
            ("%string@-2147483648%", i32::MIN),
            ("%string@0%", 0),
        ] {
            assert_semantics(
                input,
                vec![type_element(&[("string", false)], false, true, true, time)],
            );
        }

        let parsed = parse("%-*strings/location@-2%").unwrap();
        let PatternElement::TypeExpr(type_expr) = &parsed.elements[0].value else {
            panic!("expected a type expression");
        };
        assert_eq!(
            type_expr.display_with(&TEST_PLURAL_RULES).to_string(),
            "-*strings/location@-2"
        );
    }

    #[test]
    fn represents_empty_sequences_and_choice_branches() {
        use SemanticElement::*;

        assert_semantics("", vec![Empty]);
        assert_semantics(
            "(|)",
            vec![Group(vec![Choice(vec![vec![Empty], vec![Empty]])])],
        );
        assert_semantics(
            "a|",
            vec![Choice(vec![vec![Literal("a".into())], vec![Empty]])],
        );
        assert_semantics(
            "|b",
            vec![Choice(vec![vec![Empty], vec![Literal("b".into())]])],
        );
        assert_semantics(
            "(a|)",
            vec![Group(vec![Choice(vec![
                vec![Literal("a".into())],
                vec![Empty],
            ])])],
        );
        assert_semantics(
            "(|b)",
            vec![Group(vec![Choice(vec![
                vec![Empty],
                vec![Literal("b".into())],
            ])])],
        );
        assert_semantics(
            "a||b",
            vec![Choice(vec![
                vec![Literal("a".into())],
                vec![Empty],
                vec![Literal("b".into())],
            ])],
        );
    }

    #[test]
    fn parses_tags_marks_and_escaped_delimiters() {
        use SemanticElement::*;

        assert_semantics(
            "root:value (group:inside|other:branch) [option:selected]",
            vec![
                ParseTag("root".into()),
                Literal("value ".into()),
                Group(vec![Choice(vec![
                    vec![ParseTag("group".into()), Literal("inside".into())],
                    vec![ParseTag("other".into()), Literal("branch".into())],
                ])]),
                Literal(" ".into()),
                Option(vec![ParseTag("option".into()), Literal("selected".into())]),
            ],
        );
        assert_semantics(
            "1¦match|-1¦previous|0¦default",
            vec![Choice(vec![
                vec![ParseMark(1), Literal("match".into())],
                vec![ParseMark(-1), Literal("previous".into())],
                vec![ParseMark(0), Literal("default".into())],
            ])],
        );
        assert_semantics(
            ":additional",
            vec![ParseTag(String::new()), Literal("additional".into())],
        );
        assert_semantics(
            "running [(1¦below)] minecraft %string%",
            vec![
                Literal("running ".into()),
                Option(vec![Group(vec![ParseMark(1), Literal("below".into())])]),
                Literal(" minecraft ".into()),
                type_element(&[("string", false)], false, true, true, 0),
            ],
        );
        assert_semantics(
            "%entities% (is|are) (alive|1¦dead)",
            vec![
                type_element(&[("entity", true)], false, true, true, 0),
                Literal(" ".into()),
                Group(vec![Choice(vec![
                    vec![Literal("is".into())],
                    vec![Literal("are".into())],
                ])]),
                Literal(" ".into()),
                Group(vec![Choice(vec![
                    vec![Literal("alive".into())],
                    vec![ParseMark(1), Literal("dead".into())],
                ])]),
            ],
        );
        assert_semantics(
            r"tag\:value 1\¦match",
            vec![Literal("tag:value 1¦match".into())],
        );
    }

    #[test]
    fn unmatched_closing_delimiters_are_literals() {
        use SemanticElement::*;

        assert_semantics(
            "group) option] apply ->",
            vec![Literal("group) option] apply ->".into())],
        );

        let lusk_pattern = "[flower] pot[ting)| manipulat(e|ing)] [of %-itemtype%]";
        let parsed = parse(lusk_pattern).expect("registered Lusk pattern must parse");
        assert_valid_spans(lusk_pattern, &parsed.elements, None);
    }

    #[test]
    fn parse_errors_have_precise_byte_spans() {
        let input = "[unclosed";
        assert_error(
            input,
            ParseErrorKind::UnclosedBracket,
            Span::new(input.len(), input.len()),
        );

        let input = "(unclosed";
        assert_error(
            input,
            ParseErrorKind::UnclosedParenthesis,
            Span::new(input.len(), input.len()),
        );

        let input = "%unclosed";
        assert_error(
            input,
            ParseErrorKind::UnclosedTypeDelimiter,
            Span::new(input.len(), input.len()),
        );

        let input = "<unclosed";
        assert_error(
            input,
            ParseErrorKind::UnclosedRegexDelimiter,
            Span::new(input.len(), input.len()),
        );

        assert_error(
            "%-*~object@invalid%",
            ParseErrorKind::IncorrectTimeState,
            Span::new(11, 18),
        );
        assert_error(
            "%number@2147483648%",
            ParseErrorKind::IncorrectTimeState,
            Span::new(8, 18),
        );
        assert_error(
            "%number@-2147483649%",
            ParseErrorKind::IncorrectTimeState,
            Span::new(8, 19),
        );
        assert_error(
            "%object@invalid%",
            ParseErrorKind::IncorrectTimeState,
            Span::new(8, 15),
        );
        assert_error(
            "%string@%",
            ParseErrorKind::IncorrectTimeState,
            Span::new(8, 8),
        );
        assert_error(
            "not-a-number¦value",
            ParseErrorKind::InvalidParseMark,
            Span::new(0, 12),
        );
        assert_error("¦value", ParseErrorKind::InvalidParseMark, Span::new(0, 0));
        assert_error(
            "[(group]",
            ParseErrorKind::UnclosedParenthesis,
            Span::new(8, 8),
        );
        assert_error(
            "([option)",
            ParseErrorKind::UnclosedBracket,
            Span::new(9, 9),
        );

        let error = parse("%string@invalid%").unwrap_err();
        assert!(error.to_string().contains("Span { start: 8, end: 15 }"));
    }

    #[test]
    fn unclosed_errors_include_opening_delimiter_related_spans() {
        for (input, kind, opening_span) in [
            (
                "[unclosed",
                ParseErrorKind::UnclosedBracket,
                Span::new(0, 1),
            ),
            (
                "(unclosed",
                ParseErrorKind::UnclosedParenthesis,
                Span::new(0, 1),
            ),
            (
                "%unclosed",
                ParseErrorKind::UnclosedTypeDelimiter,
                Span::new(0, 1),
            ),
            (
                "<unclosed",
                ParseErrorKind::UnclosedRegexDelimiter,
                Span::new(0, 1),
            ),
            (
                "[(group]",
                ParseErrorKind::UnclosedParenthesis,
                Span::new(1, 2),
            ),
            (
                "([option)",
                ParseErrorKind::UnclosedBracket,
                Span::new(1, 2),
            ),
            (
                "((nested",
                ParseErrorKind::UnclosedParenthesis,
                Span::new(1, 2),
            ),
        ] {
            let error = parse(input).expect_err("pattern must fail");
            assert_eq!(error.kind, kind, "{input:?}");
            assert_eq!(error.span, Span::new(input.len(), input.len()));
            assert_eq!(
                error.related_spans,
                vec![RelatedSpan::new(
                    RelatedSpanKind::OpeningDelimiter,
                    opening_span
                )],
                "{input:?}"
            );
            assert!(opening_span.is_valid_for(input));
            assert!(matches!(
                opening_span.slice(input),
                Some("(" | "[" | "%" | "<")
            ));
        }
    }

    #[test]
    fn same_delimiter_nesting_reports_the_innermost_unclosed_opening() {
        for (input, kind, opening_span) in [
            (
                "((group)",
                ParseErrorKind::UnclosedParenthesis,
                Span::new(0, 1),
            ),
            (
                "(((group)",
                ParseErrorKind::UnclosedParenthesis,
                Span::new(1, 2),
            ),
            (
                "(((group))",
                ParseErrorKind::UnclosedParenthesis,
                Span::new(0, 1),
            ),
            (
                "[[option]",
                ParseErrorKind::UnclosedBracket,
                Span::new(0, 1),
            ),
            (
                "[[[option]",
                ParseErrorKind::UnclosedBracket,
                Span::new(1, 2),
            ),
            (
                "[[[option]]",
                ParseErrorKind::UnclosedBracket,
                Span::new(0, 1),
            ),
        ] {
            let error = parse(input).expect_err("nested pattern must fail");
            assert_eq!(error.kind, kind, "{input:?}");
            assert_eq!(error.span, Span::new(input.len(), input.len()), "{input:?}");
            assert_eq!(
                error.related_spans,
                vec![RelatedSpan::new(
                    RelatedSpanKind::OpeningDelimiter,
                    opening_span
                )],
                "{input:?}"
            );
        }
    }

    #[test]
    fn related_spans_preserve_utf8_offsets_and_appear_in_error_output() {
        let prefix = "日本語";
        for (delimiter, kind) in [
            ('(', ParseErrorKind::UnclosedParenthesis),
            ('[', ParseErrorKind::UnclosedBracket),
            ('%', ParseErrorKind::UnclosedTypeDelimiter),
            ('<', ParseErrorKind::UnclosedRegexDelimiter),
        ] {
            let input = format!("{prefix}{delimiter}unclosed");
            let error = parse(&input).expect_err("pattern must fail");
            assert_eq!(error.kind, kind);
            assert_eq!(error.span, Span::new(input.len(), input.len()));
            assert_eq!(
                error.related_spans[0].span,
                Span::new(prefix.len(), prefix.len() + 1)
            );
            assert!(error.related_spans[0].span.is_valid_for(&input));
        }

        let error = parse("(unclosed").unwrap_err();
        assert!(error.to_string().contains("OpeningDelimiter"));
        let unrelated = parse("%string@invalid%").unwrap_err();
        assert!(unrelated.related_spans.is_empty());
    }

    #[test]
    fn spans_preserve_original_utf8_source_ranges() {
        let input = "開始[選択|%strings%]終了";
        let parsed = parse(input).unwrap();
        assert_valid_spans(input, &parsed.elements, None);

        assert_eq!(parsed.elements[0].span.slice(input), Some("開始"));
        let option = &parsed.elements[1];
        assert_eq!(option.span.slice(input), Some("[選択|%strings%]"));

        let PatternElement::Option(option_elements) = &option.value else {
            panic!("expected an option");
        };
        let choice = &option_elements[0];
        assert_eq!(choice.span.slice(input), Some("選択|%strings%"));

        let PatternElement::Choice(branches) = &choice.value else {
            panic!("expected a choice");
        };
        assert_eq!(branches[0][0].span.slice(input), Some("選択"));
        assert_eq!(branches[1][0].span.slice(input), Some("%strings%"));
        assert_eq!(parsed.elements[2].span.slice(input), Some("終了"));

        let closing = "日本語]";
        assert_semantics(closing, vec![SemanticElement::Literal(closing.into())]);
    }

    #[test]
    fn spans_include_delimiters_and_escaped_source_text() {
        let parsed = parse("(a|[b])").unwrap();
        assert_valid_spans("(a|[b])", &parsed.elements, None);
        assert_eq!(parsed.elements[0].span, Span::new(0, 7));

        let PatternElement::Group(group) = &parsed.elements[0].value else {
            panic!("expected a group");
        };
        assert_eq!(group[0].span, Span::new(1, 6));

        let PatternElement::Choice(branches) = &group[0].value else {
            panic!("expected a choice");
        };
        assert_eq!(branches[0][0].span, Span::new(1, 2));
        assert_eq!(branches[1][0].span, Span::new(3, 6));

        let escaped = parse(r"\|\[").unwrap();
        assert_eq!(escaped.elements[0].span, Span::new(0, 4));
        assert_eq!(
            escaped.elements[0].value,
            PatternElement::Literal("|[".into())
        );

        let type_expr = parse("%string%").unwrap();
        assert_eq!(type_expr.elements[0].span, Span::new(0, 8));
        let regex = parse("<.+>").unwrap();
        assert_eq!(regex.elements[0].span, Span::new(0, 4));

        let tag = parse("tag:value").unwrap();
        assert_eq!(tag.elements[0].span, Span::new(0, 4));
        assert_eq!(tag.elements[1].span, Span::new(4, 9));

        let mark = parse("1¦value").unwrap();
        assert_eq!(mark.elements[0].span, Span::new(0, 3));
        assert_eq!(mark.elements[1].span, Span::new(3, 8));
    }

    #[test]
    fn empty_branches_use_zero_width_spans() {
        let trailing = parse("a|").unwrap();
        let PatternElement::Choice(branches) = &trailing.elements[0].value else {
            panic!("expected a choice");
        };
        assert_eq!(branches[1][0].span, Span::new(2, 2));

        let leading = parse("|b").unwrap();
        let PatternElement::Choice(branches) = &leading.elements[0].value else {
            panic!("expected a choice");
        };
        assert_eq!(branches[0][0].span, Span::new(0, 0));

        let middle = parse("a||b").unwrap();
        let PatternElement::Choice(branches) = &middle.elements[0].value else {
            panic!("expected a choice");
        };
        assert_eq!(branches[1][0].span, Span::new(2, 2));

        let empty = parse("").unwrap();
        assert_eq!(empty.elements[0].span, Span::new(0, 0));
    }

    #[test]
    fn warning_spans_use_the_same_validity_rules() {
        let input = "ラベル";
        let warning = ParseWarning {
            kind: ParseWarningKind::LabelNotSupported,
            span: Span::new(0, input.len()),
        };
        assert!(warning.span.is_valid_for(input));
        assert_eq!(warning.span.slice(input), Some(input));
        assert!(!Span::new(1, input.len()).is_valid_for(input));
        assert_eq!(Span::new(1, input.len()).slice(input), None);
    }
}
