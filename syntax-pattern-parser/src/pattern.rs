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

macro_rules! handle_scope_close {
    ($scope:expr, $expected:pat, $buffer:expr, $elements:expr, $err_kind:expr) => {{
        if matches!($scope, $expected) {
            if !$buffer.is_empty() {
                $elements.push(PatternElement::Literal(std::mem::take(&mut $buffer)));
            }
            break;
        } else {
            return Err(ParseError { kind: $err_kind });
        }
    }};
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PatternElement {
    Literal(String),
    Choice(Vec<Vec<PatternElement>>), // stuff|otherstuff
    Group(Vec<PatternElement>),       // (stuff)
    Option(Vec<PatternElement>),      // [stuff]
    Regex(String),                    // <[0-9]+>
    TypeExpr(PatternTypeExpr),        // %stuff%
    ParseTag(String),                 // tag:stuff
    ParseMark(i32),                   // 1¦stuff
    Empty,                            // (a|) -> Choice([Literal("a"), Empty])
} // todo display実装

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PatternTypeExpr {
    pub alternatives: Vec<PatternTypeAlternative>,
    pub nullable: bool,
    pub allow_literals: bool,
    pub allow_expressions: bool,
    pub time: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PatternTypeAlternative {
    pub name: String,
    pub plural: bool,
}

pub struct PatternTypeExprDisplay<'a> {
    type_expr: &'a PatternTypeExpr,
    plural_rules: &'a PluralRules,
}

impl PatternTypeExpr {
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

    let (alternatives, time) = if let Some(time_start) = body.find('@') {
        let time = body[time_start + 1..]
            .parse::<i32>()
            .map_err(|_| ParseError {
                kind: ParseErrorKind::IncorrectTimeState,
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
pub enum ParseErrorKind {
    #[error(
        "Missing closing group bracket ')'. Escape the '(' if you want to match a literal bracket: '\\('"
    )]
    UnclosedParenthesis,
    #[error(
        "Unexpected closing group bracket ')'. Escape it if you want to match a literal bracket: '\\)'"
    )]
    UnexpectedClosingParenthesis,
    #[error(
        "Missing closing optional bracket ']'. Escape the '[' if you want to match a literal bracket: '\\['"
    )]
    UnclosedBracket,
    #[error(
        "Unexpected closing optional bracket ']'. Escape it if you want to match a literal bracket: '\\]'"
    )]
    UnexpectedClosingBracket,
    #[error(
        "Missing closing type delimiter '%'. Escape the '%' if you want to match a literal percent sign: '\\%'"
    )]
    UnclosedTypeDelimiter,
    #[error(
        "Missing closing regex bracket '>'. Escape the '<' if you want to match a literal bracket: '\\<'"
    )]
    UnclosedRegexDelimiter,
    #[error(
        "Unexpected closing regex bracket '>'. Escape it if you want to match a literal bracket: '\\>'"
    )]
    UnexpectedClosingRegexDelimiter,
    #[error("Incorrect time state in type expression. It must be a 32-bit signed integer.")]
    IncorrectTimeState,
    #[error("Invalid parse mark. Text before '¦' must be a 32-bit signed integer.")]
    InvalidParseMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Hash)]
//#[error("{kind} at position {span:?}")] // todo implement
#[error("{kind}")]
pub struct ParseError {
    pub kind: ParseErrorKind,
    //pub span: Span, // todo implement
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display)]
pub enum ParseWarningKind {
    #[display(
        "Label not supported. However, it may be supported in the future (this has no effect on end users)."
    )]
    LabelNotSupported,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseWarning {
    pub kind: ParseWarningKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParseResult {
    pub elements: Vec<PatternElement>,
    pub warnings: Vec<ParseWarning>,
}

pub fn parse(input: &str, plural_rules: &PluralRules) -> Result<ParseResult, ParseError> {
    let mut chars = input.char_indices().peekable();
    parse_choice(&mut chars, Scope::Global, input, plural_rules)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Scope {
    Global,
    Group,
    Option,
}

fn parse_sequence<I: Iterator<Item = (usize, char)> + Clone>(
    chars: &mut std::iter::Peekable<I>,
    scope: Scope,
    raw_pattern: &str,
    plural_rules: &PluralRules,
) -> Result<ParseResult, ParseError> {
    let mut elements = Vec::new();
    let mut buffer = String::new();

    let mut warnings = Vec::new();

    while let Some(&(i, ch)) = chars.peek() {
        match ch {
            '(' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(std::mem::take(&mut buffer)));
                }
                chars.next(); // consume '('

                match parse_choice(chars, Scope::Group, raw_pattern, plural_rules) {
                    Ok(group) => {
                        warnings.extend(group.warnings);
                        elements.push(PatternElement::Group(group.elements));
                    }
                    Err(ParseError {
                        kind: ParseErrorKind::UnexpectedClosingBracket,
                        ..
                    }) if scope == Scope::Option => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedClosingParenthesis,
                        });
                    }
                    Err(e) => return Err(e),
                }
            }
            ')' => {
                handle_scope_close!(
                    scope,
                    Scope::Group,
                    buffer,
                    elements,
                    ParseErrorKind::UnexpectedClosingParenthesis
                );
            }
            '[' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(std::mem::take(&mut buffer)));
                }
                chars.next(); // consume '['

                match parse_choice(chars, Scope::Option, raw_pattern, plural_rules) {
                    Ok(option) => {
                        warnings.extend(option.warnings);
                        elements.push(PatternElement::Option(option.elements));
                    }
                    Err(ParseError {
                        kind: ParseErrorKind::UnexpectedClosingParenthesis,
                        ..
                    }) if scope == Scope::Group => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnclosedBracket,
                        });
                    }
                    Err(e) => return Err(e),
                }
            }
            ']' => {
                handle_scope_close!(
                    scope,
                    Scope::Option,
                    buffer,
                    elements,
                    ParseErrorKind::UnexpectedClosingBracket
                );
            }
            '<' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(std::mem::take(&mut buffer)));
                }
                chars.next(); // consume '<'

                let start = i + '<'.len_utf8();
                if let Some(rel) = raw_pattern[start..].find('>') {
                    let end = start + rel;

                    let regex_slice = &raw_pattern[start..end];

                    // イテレータを '>' の直後まで一気に進める
                    consume_until!(chars, end);

                    elements.push(PatternElement::Regex(regex_slice.to_string()));
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnclosedRegexDelimiter,
                    });
                }
            }
            '>' => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedClosingRegexDelimiter,
                });
            }
            '%' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(std::mem::take(&mut buffer)));
                }
                chars.next(); // consume '%'

                let start = i + '%'.len_utf8();
                if let Some(rel) = raw_pattern[start..].find('%') {
                    let end = start + rel;

                    let type_expr_str = &raw_pattern[start..end];

                    // イテレータを '%' の直後まで一気に進める
                    consume_until!(chars, end);

                    elements.push(PatternElement::TypeExpr(parse_pattern_type_expr(
                        type_expr_str,
                        plural_rules,
                    )?));
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnclosedTypeDelimiter,
                    });
                }
            }
            '|' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(std::mem::take(&mut buffer)));
                }
                break;
            }
            '\\' => {
                chars.next(); // consume '\'
                if let Some(&(_, c)) = chars.peek() {
                    buffer.push(c);
                    chars.next(); // consume escaped character
                } else {
                    buffer.push('\\');
                }
            }
            ':' => {
                elements.push(PatternElement::ParseTag(std::mem::take(&mut buffer)));
                chars.next();
            }
            '¦' => {
                let mark = std::mem::take(&mut buffer)
                    .parse::<i32>()
                    .map_err(|_| ParseError {
                        kind: ParseErrorKind::InvalidParseMark,
                    })?;
                elements.push(PatternElement::ParseMark(mark));
                chars.next();
            }
            _ => {
                buffer.push(ch);
                chars.next();
            }
        }
    }

    if !buffer.is_empty() {
        elements.push(PatternElement::Literal(buffer));
    }

    Ok(ParseResult { elements, warnings })
}

fn parse_choice<I: Iterator<Item = (usize, char)> + Clone>(
    chars: &mut std::iter::Peekable<I>,
    scope: Scope,
    raw_pattern: &str,
    plural_rules: &PluralRules,
) -> Result<ParseResult, ParseError> {
    let mut branches: Vec<Vec<PatternElement>> = Vec::new();
    let mut closed = false; // 対応する閉じ括弧を消費できたか

    let mut warnings = Vec::new();

    loop {
        let seq = parse_sequence(chars, scope, raw_pattern, plural_rules)?;
        warnings.extend(seq.warnings);
        if !seq.elements.is_empty() {
            branches.push(seq.elements);
        } else {
            branches.push(vec![PatternElement::Empty]);
        }

        match chars.peek() {
            Some(&(_, '|')) => {
                chars.next(); // 次の分岐へ
            }
            Some(&(_, ')')) if scope == Scope::Group => {
                chars.next(); // 対応する閉じ括弧を消費
                closed = true;
                break;
            }
            Some(&(_, ']')) if scope == Scope::Option => {
                chars.next(); // 対応する閉じ括弧を消費
                closed = true;
                break;
            }
            None => break,
            _ => break,
        }
    }

    match scope {
        Scope::Group if !closed => {
            return Err(ParseError {
                kind: ParseErrorKind::UnclosedParenthesis,
            });
        }
        Scope::Option if !closed => {
            return Err(ParseError {
                kind: ParseErrorKind::UnclosedBracket,
            });
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
            elements: vec![PatternElement::Choice(branches)],
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

    use PatternElement::*;

    fn type_element(
        alternatives: &[(&str, bool)],
        nullable: bool,
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
    ) -> PatternElement {
        TypeExpr(PatternTypeExpr {
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

    mod simple_tests {
        use super::*;
        #[test]
        fn parse_literal() {
            let literal = parse("literal");
            assert_eq!(
                literal,
                Ok(ParseResult {
                    elements: vec![Literal("literal".to_string())],
                    warnings: vec![]
                })
            );
        }

        #[test]
        fn parse_choice() {
            let choice = parse("(choice1|choice2)");
            assert_eq!(
                choice,
                Ok(ParseResult {
                    elements: vec![Group(vec![Choice(vec![
                        vec![Literal("choice1".to_string())],
                        vec![Literal("choice2".to_string())],
                    ])])],
                    warnings: vec![]
                })
            );
        }

        #[test]
        fn parse_group() {
            let group = parse("(group)");
            assert_eq!(
                group,
                Ok(ParseResult {
                    elements: vec![Group(vec![Literal("group".to_string())])],
                    warnings: vec![]
                })
            );
        }

        #[test]
        fn parse_option() {
            let option = parse("[option]");
            assert_eq!(
                option,
                Ok(ParseResult {
                    elements: vec![Option(vec![Literal("option".to_string())])],
                    warnings: vec![]
                })
            );
        }

        #[test]
        fn parse_regex() {
            let regex = parse("<[0-9]+>");
            assert_eq!(
                regex,
                Ok(ParseResult {
                    elements: vec![Regex("[0-9]+".to_string())],
                    warnings: vec![]
                })
            );
        }

        mod type_expr {
            use super::*;

            #[test]
            fn simple() {
                assert_eq!(
                    parse("%string%"),
                    Ok(ParseResult {
                        elements: vec![type_element(&[("string", false)], false, true, true, 0)],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn literal_only() {
                assert_eq!(
                    parse("%*string%"),
                    Ok(ParseResult {
                        elements: vec![type_element(&[("string", false)], false, true, false, 0)],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn expression_only() {
                assert_eq!(
                    parse("%~string%"),
                    Ok(ParseResult {
                        elements: vec![type_element(&[("string", false)], false, false, true, 0)],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn nullable() {
                assert_eq!(
                    parse("%-string%"),
                    Ok(ParseResult {
                        elements: vec![type_element(&[("string", false)], true, true, true, 0)],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn modifiers_apply_to_all_alternatives() {
                assert_eq!(
                    parse("%-*~strings/locations@12%"),
                    Ok(ParseResult {
                        elements: vec![type_element(
                            &[("string", true), ("location", true)],
                            true,
                            false,
                            false,
                            12,
                        )],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn modifiers_after_slash_are_part_of_the_type_name() {
                assert_eq!(
                    parse("%string/*numbers/-texts%"),
                    Ok(ParseResult {
                        elements: vec![type_element(
                            &[("string", false), ("*number", true), ("-text", true)],
                            false,
                            true,
                            true,
                            0,
                        )],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn plural_alternatives_use_skript_english_rules() {
                assert_eq!(
                    parse("%children/aliases/axes/sheep/sheeps/dummyfixturepeople%"),
                    Ok(ParseResult {
                        elements: vec![type_element(
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
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn time_uses_the_full_i32_range() {
                assert_eq!(
                    parse("%string@2147483647%"),
                    Ok(ParseResult {
                        elements: vec![type_element(
                            &[("string", false)],
                            false,
                            true,
                            true,
                            i32::MAX,
                        )],
                        warnings: vec![]
                    })
                );
                assert_eq!(
                    parse("%string@-2147483648%"),
                    Ok(ParseResult {
                        elements: vec![type_element(
                            &[("string", false)],
                            false,
                            true,
                            true,
                            i32::MIN,
                        )],
                        warnings: vec![]
                    })
                );
                assert_eq!(
                    parse("%string@0%"),
                    Ok(ParseResult {
                        elements: vec![type_element(&[("string", false)], false, true, true, 0)],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn display_reconstructs_type_expression() {
                let parsed = parse("%-*strings/location@-2%").unwrap();
                let TypeExpr(type_expr) = &parsed.elements[0] else {
                    panic!("expected a type expression");
                };
                assert_eq!(
                    type_expr.display_with(&TEST_PLURAL_RULES).to_string(),
                    "-*strings/location@-2"
                );
            }
        }

        mod empty {
            use super::*;

            #[test]
            fn simple() {
                let empty_simple = parse("");
                assert_eq!(
                    empty_simple,
                    Ok(ParseResult {
                        elements: vec![Empty],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn choice_trailing() {
                let empty_choice = parse("(a|)");
                assert_eq!(
                    empty_choice,
                    Ok(ParseResult {
                        elements: vec![Group(vec![Choice(vec![
                            vec![Literal("a".to_string())],
                            vec![Empty],
                        ])])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn choice_leading() {
                let empty_choice2 = parse("(|b)");
                assert_eq!(
                    empty_choice2,
                    Ok(ParseResult {
                        elements: vec![Group(vec![Choice(vec![
                            vec![Empty],
                            vec![Literal("b".to_string())],
                        ])])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn choice_both() {
                let empty_choice3 = parse("(|)");
                assert_eq!(
                    empty_choice3,
                    Ok(ParseResult {
                        elements: vec![Group(vec![Choice(vec![vec![Empty], vec![Empty]])])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn global_choice_empty_branches() {
                assert_eq!(
                    parse("a|"),
                    Ok(ParseResult {
                        elements: vec![Choice(vec![vec![Literal("a".to_string())], vec![Empty],])],
                        warnings: vec![],
                    })
                );

                assert_eq!(
                    parse("|b"),
                    Ok(ParseResult {
                        elements: vec![Choice(vec![vec![Empty], vec![Literal("b".to_string())],])],
                        warnings: vec![],
                    })
                );

                assert_eq!(
                    parse("a||b"),
                    Ok(ParseResult {
                        elements: vec![Choice(vec![
                            vec![Literal("a".to_string())],
                            vec![Empty],
                            vec![Literal("b".to_string())],
                        ])],
                        warnings: vec![],
                    })
                );
            }
        }
    }

    #[test]
    fn parse_tests() {
        use PatternElement::*;

        let syntax = parse("(absolute|complete) path of %string%");
        assert_eq!(
            syntax,
            Ok(ParseResult {
                elements: vec![
                    Group(vec![Choice(vec![
                        vec![Literal("absolute".to_string())],
                        vec![Literal("complete".to_string())],
                    ])]),
                    Literal(" path of ".to_string()),
                    type_element(&[("string", false)], false, true, true, 0),
                ],
                warnings: vec![]
            })
        );

        let syntax = parse("[local] %skript types% property condition <pattern>");
        assert_eq!(
            syntax,
            Ok(ParseResult {
                elements: vec![
                    Option(vec![Literal("local".to_string())]),
                    Literal(" ".to_string()),
                    type_element(&[("skript type", true)], false, true, true, 0),
                    Literal(" property condition ".to_string()),
                    Regex("pattern".to_string()),
                ],
                warnings: vec![]
            })
        );

        let syntax = parse("<.+> \\|\\| <.+>");
        assert_eq!(
            syntax,
            Ok(ParseResult {
                elements: vec![
                    Regex(".+".to_string()),
                    Literal(" || ".to_string()),
                    Regex(".+".to_string()),
                ],
                warnings: vec![]
            })
        );

        let syntax = parse("folder|dir|box");
        assert_eq!(
            syntax,
            Ok(ParseResult {
                elements: vec![Choice(vec![
                    vec![Literal("folder".to_string())],
                    vec![Literal("dir".to_string())],
                    vec![Literal("box".to_string())],
                ])],
                warnings: vec![],
            })
        );

        let syntax = parse("[foo|bar]");
        assert_eq!(
            syntax,
            Ok(ParseResult {
                elements: vec![Option(vec![Choice(vec![
                    vec![Literal("foo".to_string())],
                    vec![Literal("bar".to_string())],
                ])])],
                warnings: vec![],
            })
        );

        let syntax = parse("active[ |-](group|model)[s]");
        assert_eq!(
            syntax,
            Ok(ParseResult {
                elements: vec![
                    Literal("active".to_string()),
                    Option(vec![Choice(vec![
                        vec![Literal(" ".to_string())],
                        vec![Literal("-".to_string())],
                    ])]),
                    Group(vec![Choice(vec![
                        vec![Literal("group".to_string())],
                        vec![Literal("model".to_string())],
                    ])]),
                    Option(vec![Literal("s".to_string())]),
                ],
                warnings: vec![],
            })
        );
    }

    #[cfg(test)]
    mod error_warn_tests {
        use super::*;

        #[test]
        fn unclosed_bracket() {
            let pattern = parse("[unclosed");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedBracket,
                })
            );

            let pattern = parse("start [unclosed (group)");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedBracket,
                })
            );
        }

        #[test]
        fn unclosed_parenthesis() {
            let pattern = parse("(unclosed");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedParenthesis,
                })
            );

            let pattern = parse("start (unclosed [option]");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedParenthesis,
                })
            );
        }

        #[test]
        fn incorrect_time_state() {
            for pattern in [
                "%string@%",
                "%string@invalid%",
                "start %number@2147483648%",
                "%number@-2147483649%",
            ] {
                assert_eq!(
                    parse(pattern),
                    Err(ParseError {
                        kind: ParseErrorKind::IncorrectTimeState,
                    })
                );
            }
        }

        #[test]
        fn unclosed_type_delimiter() {
            let pattern = parse("%unclosed");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedTypeDelimiter,
                })
            );

            let pattern = parse("start %unclosed/string");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedTypeDelimiter,
                })
            );
        }

        #[test]
        fn unclosed_regex_delimiter() {
            let pattern = parse("<unclosed");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedRegexDelimiter,
                })
            );

            let pattern = parse("start <unclosed [any]");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedRegexDelimiter,
                })
            );
        }

        #[test]
        fn unclosed_parenthesis_in_option() {
            let pattern = parse("[(unclosed group]");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnexpectedClosingParenthesis,
                })
            );
        }

        #[test]
        fn unclosed_bracket_in_group() {
            let pattern = parse("([unclosed option)");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnclosedBracket,
                })
            );
        }

        #[test]
        fn unexpected_closing_parenthesis() {
            let pattern = parse("unclosed group)");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnexpectedClosingParenthesis,
                })
            );
        }

        #[test]
        fn unexpected_closing_bracket() {
            let pattern = parse("unclosed option]");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnexpectedClosingBracket,
                })
            );
        }

        #[test]
        fn unexpected_closing_regex_delimiter() {
            let pattern = parse("unclosed regex>");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnexpectedClosingRegexDelimiter,
                })
            );
        }
    }

    #[cfg(test)]
    mod other_tests {
        use super::*;

        #[test]
        fn parse_tags_in_all_scopes_and_choices() {
            let pattern = parse("root:value (group:inside|other:branch) [option:selected]");
            assert_eq!(
                pattern,
                Ok(ParseResult {
                    elements: vec![
                        ParseTag("root".to_string()),
                        Literal("value ".to_string()),
                        Group(vec![Choice(vec![
                            vec![ParseTag("group".to_string()), Literal("inside".to_string()),],
                            vec![ParseTag("other".to_string()), Literal("branch".to_string()),],
                        ])]),
                        Literal(" ".to_string()),
                        Option(vec![
                            ParseTag("option".to_string()),
                            Literal("selected".to_string()),
                        ]),
                    ],
                    warnings: vec![],
                })
            );

            assert_eq!(
                parse(":additional"),
                Ok(ParseResult {
                    elements: vec![ParseTag(String::new()), Literal("additional".to_string()),],
                    warnings: vec![],
                })
            );
        }

        #[test]
        fn parse_marks_including_zero_and_negative_values() {
            assert_eq!(
                parse("1¦match|-1¦previous|0¦default"),
                Ok(ParseResult {
                    elements: vec![Choice(vec![
                        vec![ParseMark(1), Literal("match".to_string())],
                        vec![ParseMark(-1), Literal("previous".to_string())],
                        vec![ParseMark(0), Literal("default".to_string())],
                    ])],
                    warnings: vec![],
                })
            );
        }

        #[test]
        fn escaped_tag_and_mark_delimiters_are_literals() {
            assert_eq!(
                parse(r"tag\:value 1\¦match"),
                Ok(ParseResult {
                    elements: vec![Literal("tag:value 1¦match".to_string())],
                    warnings: vec![],
                })
            );
        }

        #[test]
        fn invalid_parse_marks_are_rejected() {
            for pattern in ["not-a-number¦value", "¦value", "2147483648¦overflow"] {
                assert_eq!(
                    parse(pattern),
                    Err(ParseError {
                        kind: ParseErrorKind::InvalidParseMark,
                    })
                );
            }
        }

        #[test]
        fn parses_generator_patterns_with_marks() {
            assert_eq!(
                parse("running [(1¦below)] minecraft %string%"),
                Ok(ParseResult {
                    elements: vec![
                        Literal("running ".to_string()),
                        Option(vec![Group(vec![
                            ParseMark(1),
                            Literal("below".to_string()),
                        ])]),
                        Literal(" minecraft ".to_string()),
                        type_element(&[("string", false)], false, true, true, 0),
                    ],
                    warnings: vec![],
                })
            );

            assert_eq!(
                parse("%entities% (is|are) (alive|1¦dead)"),
                Ok(ParseResult {
                    elements: vec![
                        type_element(&[("entity", true)], false, true, true, 0),
                        Literal(" ".to_string()),
                        Group(vec![Choice(vec![
                            vec![Literal("is".to_string())],
                            vec![Literal("are".to_string())],
                        ])]),
                        Literal(" ".to_string()),
                        Group(vec![Choice(vec![
                            vec![Literal("alive".to_string())],
                            vec![ParseMark(1), Literal("dead".to_string())],
                        ])]),
                    ],
                    warnings: vec![],
                })
            );
        }
    }
}
