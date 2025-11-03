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
    TypeExpr(Vec<PatternTypeExpr>),   // %stuff%
    Empty,                            // (a|) -> Choice([Literal("a"), Empty])
} // todo display実装

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PatternTypeExpr {
    pub name: String,
    pub literal: bool,                           // %*stuff%
    pub non_literal: bool,                       // %~stuff%
    pub nullable: bool,                          // %-stuff%
    pub time_state: Option<std::num::NonZeroI8>, // %stuff@d%
}
impl std::fmt::Display for PatternTypeExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.literal {
            write!(f, "*")?;
        }
        if self.non_literal {
            write!(f, "~")?;
        }
        if self.nullable {
            write!(f, "-")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(ts) = self.time_state {
            write!(f, "@{}", ts.get())?; //todo
        }
        Ok(())
    }
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
    #[error(
        "Cannot use the pipe character '|' outside of groups. Escape it if you want to match a literal pipe: '\\|'"
    )]
    UnexpectedPipeOutsideGroup,
    #[error("Incorrect time state in type expression. It must be either @1 or @-1.")]
    IncorrectTimeState,
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

pub fn parse(input: &str) -> Result<ParseResult, ParseError> {
    let mut chars = input.char_indices().peekable();
    parse_choice(&mut chars, Scope::Global, input)
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

                match parse_choice(chars, Scope::Group, raw_pattern) {
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

                match parse_choice(chars, Scope::Option, raw_pattern) {
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

                    let types = type_expr_str.split('/');
                    let mut type_exprs = Vec::new();
                    for t in types {
                        let mut a = t;
                        let mut literal = false;
                        let mut non_literal = false;
                        let mut nullable = false;
                        let mut time_state = None;
                        loop {
                            if a.starts_with('*') {
                                literal = true;
                                a = &a[1..];
                            } else if a.starts_with('~') {
                                non_literal = true;
                                a = &a[1..];
                            } else if a.starts_with('-') {
                                nullable = true;
                                a = &a[1..];
                            } else if let Some(at_pos) = a.find('@') {
                                if let Ok(ts) = a[at_pos + 1..].parse::<i8>() {
                                    if !(ts == -1 || ts == 1) {
                                        return Err(ParseError {
                                            kind: ParseErrorKind::IncorrectTimeState,
                                        });
                                    }
                                    time_state = std::num::NonZeroI8::new(ts);
                                    a = &a[..at_pos];
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        type_exprs.push(PatternTypeExpr {
                            name: a.to_string(),
                            literal,
                            non_literal,
                            nullable,
                            time_state,
                        });
                    }
                    elements.push(PatternElement::TypeExpr(type_exprs));
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnclosedTypeDelimiter,
                    });
                }
            }
            '|' => {
                handle_scope_close!(
                    scope,
                    Scope::Group,
                    buffer,
                    elements,
                    ParseErrorKind::UnexpectedPipeOutsideGroup
                );
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
            ':' | '¦' if scope == Scope::Group => {
                // todo ラベル付きグループのサポート
                // skript-hubではpatternに変換される際にラベルが消されるため、実装の優先度は低い
                // (詳しくはこちら: https://github.com/SkriptHub/SkriptHubDocsTool/blob/0e0ef70a370227672301a51e3125dc7fc1663278/src/main/kotlin/net/skripthub/docstool/documentation/GenerateSyntax.kt#L265-L288)
                // そのため今はあっても破棄している
                // issue: https://github.com/nlaocs/Skript-LSP/issues/5
                if !buffer.is_empty() {
                    buffer.clear();
                }
                warnings.push(ParseWarning {
                    kind: ParseWarningKind::LabelNotSupported,
                    span: Span {
                        start: i,
                        end: i + ch.len_utf8(),
                    },
                });
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
) -> Result<ParseResult, ParseError> {
    let mut branches: Vec<Vec<PatternElement>> = Vec::new();
    let mut closed = false; // 対応する閉じ括弧を消費できたか

    let mut warnings = Vec::new();

    loop {
        let seq = parse_sequence(chars, scope, raw_pattern)?;
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

    use PatternElement::*;

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
                let type_expr = parse("%string%");
                assert_eq!(
                    type_expr,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![PatternTypeExpr {
                            name: "string".to_string(),
                            literal: false,
                            non_literal: false,
                            nullable: false,
                            time_state: None,
                        }])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn literal() {
                let t = parse("%*string%");
                assert_eq!(
                    t,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![PatternTypeExpr {
                            name: "string".to_string(),
                            literal: true,
                            non_literal: false,
                            nullable: false,
                            time_state: None,
                        }])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn non_literal() {
                let t = parse("%~string%");
                assert_eq!(
                    t,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![PatternTypeExpr {
                            name: "string".to_string(),
                            literal: false,
                            non_literal: true,
                            nullable: false,
                            time_state: None,
                        }])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn nullable() {
                let t = parse("%-string%");
                assert_eq!(
                    t,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![PatternTypeExpr {
                            name: "string".to_string(),
                            literal: false,
                            non_literal: false,
                            nullable: true,
                            time_state: None,
                        }])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn with_time_state() {
                let t = parse("%string@1%");
                assert_eq!(
                    t,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![PatternTypeExpr {
                            name: "string".to_string(),
                            literal: false,
                            non_literal: false,
                            nullable: false,
                            time_state: std::num::NonZeroI8::new(1),
                        }])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn multiple() {
                let t = parse("%string/*number/-text%");
                assert_eq!(
                    t,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![
                            PatternTypeExpr {
                                name: "string".to_string(),
                                literal: false,
                                non_literal: false,
                                nullable: false,
                                time_state: None,
                            },
                            PatternTypeExpr {
                                name: "number".to_string(),
                                literal: true,
                                non_literal: false,
                                nullable: false,
                                time_state: None,
                            },
                            PatternTypeExpr {
                                name: "text".to_string(),
                                literal: false,
                                non_literal: false,
                                nullable: true,
                                time_state: None,
                            },
                        ])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn multiple_with_time() {
                let t = parse("%*string/~number/-text@-1%");
                assert_eq!(
                    t,
                    Ok(ParseResult {
                        elements: vec![TypeExpr(vec![
                            PatternTypeExpr {
                                name: "string".to_string(),
                                literal: true,
                                non_literal: false,
                                nullable: false,
                                time_state: None,
                            },
                            PatternTypeExpr {
                                name: "number".to_string(),
                                literal: false,
                                non_literal: true,
                                nullable: false,
                                time_state: None,
                            },
                            PatternTypeExpr {
                                name: "text".to_string(),
                                literal: false,
                                non_literal: false,
                                nullable: true,
                                time_state: std::num::NonZeroI8::new(-1),
                            },
                        ])],
                        warnings: vec![]
                    })
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
                    TypeExpr(vec![PatternTypeExpr {
                        name: "string".to_string(),
                        literal: false,
                        non_literal: false,
                        nullable: false,
                        time_state: None,
                    }]),
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
                    TypeExpr(vec![PatternTypeExpr {
                        name: "skript types".to_string(),
                        literal: false,
                        non_literal: false,
                        nullable: false,
                        time_state: None,
                    }]),
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
            Err(ParseError {
                kind: ParseErrorKind::UnexpectedPipeOutsideGroup,
            })
        );

        let syntax = parse("active[ |-](group|model)[s]");
        assert_eq!(
            syntax,
            Err(ParseError {
                kind: ParseErrorKind::UnexpectedPipeOutsideGroup,
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
            let pattern = parse("%string@0%");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::IncorrectTimeState,
                })
            );

            let pattern = parse("start %number@2%");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::IncorrectTimeState,
                })
            );
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

        #[test]
        fn unexpected_pipe_in_global() {
            let pattern = parse("no|group");
            assert_eq!(
                pattern,
                Err(ParseError {
                    kind: ParseErrorKind::UnexpectedPipeOutsideGroup,
                })
            );
        }
    }

    #[cfg(test)]
    mod other_tests {
        use super::*;

        #[test]
        fn labelled_group() {
            let pattern = parse("(label:group)");
            assert_eq!(
                pattern,
                Ok(ParseResult {
                    elements: vec![Group(vec![Literal("group".to_string())])],
                    warnings: vec![ParseWarning {
                        kind: ParseWarningKind::LabelNotSupported,
                        span: Span { start: 6, end: 7 },
                    }]
                })
            );

            let pattern = parse("(¦group)");
            assert_eq!(
                pattern,
                Ok(ParseResult {
                    elements: vec![Group(vec![Literal("group".to_string())])],
                    warnings: vec![ParseWarning {
                        kind: ParseWarningKind::LabelNotSupported,
                        span: Span { start: 1, end: 3 },
                    }]
                })
            );
        }
    }
}
