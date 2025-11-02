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
    #[error("Unclosed parenthesis '('")]
    UnclosedParenthesis,
    #[error("Unclosed bracket '['")]
    UnclosedBracket,
    #[error("Incorrect time state in type expression")]
    IncorrectTimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Hash)]
#[error("{kind} at position {span:?}")] // todo implement
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display)]
pub enum ParseWarningKind {
    #[display("Unclosed type delimiter '%'")]
    UnclosedTypeDelimiter,
    #[display("Unclosed regex delimiter '>'")]
    UnclosedRegexDelimiter,
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
    let mut chars = input.chars().peekable();
    Ok(parse_choice(&mut chars, Scope::Global, input)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Scope {
    Global,
    Group,
    Option,
}

fn parse_sequence<I: Iterator<Item = char> + Clone>(
    chars: &mut std::iter::Peekable<I>,
    scope: Scope,
    raw_pattern: &str,
) -> Result<ParseResult, ParseError> {
    let mut elements = Vec::new();
    let mut buffer = String::new();

    let mut warnings = Vec::new();

    while let Some(&ch) = chars.peek() {
        match ch {
            '(' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                chars.next(); // consume '('
                let group = parse_choice(chars, Scope::Group, raw_pattern)?;
                warnings.extend(group.warnings);
                elements.push(PatternElement::Group(group.elements));
            }
            ')' if scope == Scope::Group => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                break;
            }
            '[' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                chars.next(); // consume '['
                // [(] の場合はOption(Literal("("))として扱う todo typeをskript patternにしたときのバグの可能性があるので必要がどうかが不明
                if chars.peek() == Some(&'(') {
                    let mut chars_clone = chars.clone();
                    chars_clone.next(); // consume '('
                    if chars_clone.peek() == Some(&']') {
                        chars.next(); // consume '('
                        chars.next(); // consume ']'
                        elements.push(PatternElement::Option(vec![PatternElement::Literal(
                            "(".to_string(),
                        )]));
                        continue;
                    }
                }

                let option = parse_choice(chars, Scope::Option, raw_pattern)?;
                warnings.extend(option.warnings);
                elements.push(PatternElement::Option(option.elements));
            }
            ']' if scope == Scope::Option => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                break;
            }
            '<' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                chars.next(); // consume '<'
                let mut regex = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '>' {
                        break;
                    }
                    regex.push(c);
                    chars.next();
                }
                if chars.peek() == Some(&'>') {
                    chars.next(); // consume '>'
                    elements.push(PatternElement::Regex(regex));
                } else {
                    // todo 今のままでは不完全な正規表現がリテラルとして扱われるが、[]などが出てきてもリテラルとして扱われてしまう
                    // https://github.com/nlaocs/Skript-LSP/issues/3
                    elements.push(PatternElement::Literal(format!("<{}", regex)));
                }
            }
            '%' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                chars.next(); // consume '%'
                let mut type_expr_str = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '%' {
                        break;
                    }
                    type_expr_str.push(c);
                    chars.next();
                }
                if chars.peek() == Some(&'%') {
                    chars.next(); // consume '%'
                    if cfg!(debug_assertions) && type_expr_str.contains(' ') {
                        eprintln!(
                            "Warning: Type expression contains spaces in pattern: {}",
                            raw_pattern
                        );
                    }
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
                    elements.push(PatternElement::Literal(format!("%{}", type_expr_str)));
                }
            }
            '|' => {
                if !buffer.is_empty() {
                    elements.push(PatternElement::Literal(buffer.clone()));
                    buffer.clear();
                }
                break;
            }
            '\\' => {
                chars.next(); // consume '\'
                if let Some(&next_ch) = chars.peek() {
                    buffer.push(next_ch);
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
                if !buffer.is_empty() {
                    buffer.clear();
                }
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

fn parse_choice<I: Iterator<Item = char> + Clone>(
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
            Some('|') => {
                chars.next(); // 次の分岐へ
            }
            Some(')') if scope == Scope::Group => {
                chars.next(); // 対応する閉じ括弧を消費
                closed = true;
                break;
            }
            Some(']') if scope == Scope::Option => {
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
                span: Span {
                    start: 0, // todo
                    end: raw_pattern.len(),
                },
            });
        }
        Scope::Option if !closed => {
            return Err(ParseError {
                kind: ParseErrorKind::UnclosedBracket,
                span: Span {
                    start: 0, // todo
                    end: raw_pattern.len(),
                },
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
            let choice = parse("choice1|choice2");
            assert_eq!(
                choice,
                Ok(ParseResult {
                    elements: vec![Choice(vec![
                        vec![Literal("choice1".to_string())],
                        vec![Literal("choice2".to_string())],
                    ])],
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
                let empty_choice = parse("a|");
                assert_eq!(
                    empty_choice,
                    Ok(ParseResult {
                        elements: vec![Choice(vec![vec![Literal("a".to_string())], vec![Empty],])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn choice_leading() {
                let empty_choice2 = parse("|b");
                assert_eq!(
                    empty_choice2,
                    Ok(ParseResult {
                        elements: vec![Choice(vec![vec![Empty], vec![Literal("b".to_string())],])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn choice_both() {
                let empty_choice3 = parse("|");
                assert_eq!(
                    empty_choice3,
                    Ok(ParseResult {
                        elements: vec![Choice(vec![vec![Empty], vec![Empty]])],
                        warnings: vec![]
                    })
                );
            }

            #[test]
            fn group_option() {
                let empty_group_option = parse("(|[|])");
                assert_eq!(
                    empty_group_option,
                    Ok(ParseResult {
                        elements: vec![Group(vec![Choice(vec![
                            vec![Empty],
                            vec![Option(vec![Choice(vec![vec![Empty], vec![Empty]])])],
                        ])])],
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
            Ok(ParseResult {
                elements: vec![Choice(vec![
                    vec![Literal("folder".to_string())],
                    vec![Literal("dir".to_string())],
                    vec![Literal("box".to_string())],
                ])],
                warnings: vec![]
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
                warnings: vec![]
            })
        );
    }

    #[cfg(test)]
    mod error_tests {
        use super::*;

        #[test]
        fn unclosed_type_delimiter() {
            let pattern = parse("%unclosed");
            assert_eq!(
                pattern,
                Ok(ParseResult {
                    elements: vec![Literal("%unclosed".to_string())],
                    warnings: vec![]
                })
            );
        }

        #[test]
        fn unclosed_regex_delimiter() {
            let pattern = parse("<unclosed");
            assert_eq!(
                pattern,
                Ok(ParseResult {
                    elements: vec![Literal("<unclosed".to_string())],
                    warnings: vec![]
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
                    warnings: vec![]
                })
            );

            let pattern = parse("(¦group)");
            assert_eq!(
                pattern,
                Ok(ParseResult {
                    elements: vec![Group(vec![Literal("group".to_string())])],
                    warnings: vec![]
                })
            );
        }
    }
}
