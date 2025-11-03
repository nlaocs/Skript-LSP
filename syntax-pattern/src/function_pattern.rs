#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnParseResult {
    pub inner: Function,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Function {
    pub name: String,
    pub args: Vec<Arg>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Arg {
    pub name: String,
    pub arg_type: String,
    pub default_expression: Option<String>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Hash)]
#[error("{kind}")]
pub struct FnParseError {
    pub kind: FnParseErrorKind,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Hash)]
pub enum FnParseErrorKind {
    #[error("Invalid function name: {0:?}")] // todo
    InvalidFunctionName(InvalidFunctionNameKind),
    #[error("Function name is empty")]
    EmptyFunctionName,
    #[error("Invalid argument")]
    InvalidArgument, // todo 分けてもいいかも function test(,) や function test(a: s:tring),など
    #[error("Unclosed parenthesis")]
    UnclosedParenthesis,
    #[error("Unclosed string")]
    UnclosedString,
    #[error("Function name contains space")]
    FunctionNameContainsSpace,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InvalidFunctionNameKind {
    UnexpectedFirstCharacter,
    UnexpectedCharacter,
}

pub fn parse(input: &str) -> Result<FnParseResult, FnParseError> {
    let mut chars = input.char_indices().peekable();

    let mut buffer = String::new();
    let mut first_char = true;

    let mut name = String::new();
    let mut args = Vec::new();

    while let Some(&(_, ch)) = chars.peek() {
        match ch {
            '(' => {
                chars.next();
                if buffer.is_empty() {
                    return Err(FnParseError {
                        kind: FnParseErrorKind::EmptyFunctionName,
                    });
                }
                name = buffer.clone();
                break;
            }
            ch if first_char => {
                if ch.is_alphabetic() || ch == '_' {
                    buffer.push(ch);
                    chars.next();
                    first_char = false;
                } else {
                    return Err(FnParseError {
                        kind: FnParseErrorKind::InvalidFunctionName(
                            InvalidFunctionNameKind::UnexpectedFirstCharacter,
                        ),
                    });
                }
            }
            ch if ch.is_alphanumeric() || ch == '_' || ch.is_ascii_digit() => {
                buffer.push(ch);
                chars.next();
            }
            ch if ch.is_whitespace() => {
                return Err(FnParseError {
                    kind: FnParseErrorKind::FunctionNameContainsSpace,
                });
            }
            _ => {
                return Err(FnParseError {
                    kind: FnParseErrorKind::InvalidFunctionName(
                        InvalidFunctionNameKind::UnexpectedCharacter,
                    ),
                });
            }
        }
    }

    let mut abstract_args = Vec::new();
    let mut is_closed = false;

    buffer.clear();
    let mut last_char_was_comma = false;

    // この時点でabc(d: string)のd:string)の部分だけになっている
    while let Some(&(_, ch)) = chars.peek() {
        match ch {
            ')' => {
                chars.next();
                let arg_str = buffer.trim();
                if arg_str.is_empty() && last_char_was_comma {
                    return Err(FnParseError {
                        kind: FnParseErrorKind::InvalidArgument,
                    });
                }

                if !arg_str.is_empty() {
                    abstract_args.push(arg_str.to_string());
                }
                is_closed = true;
                break;
            }
            ',' => {
                let arg_str = buffer.trim();
                if !arg_str.is_empty() {
                    abstract_args.push(arg_str.to_string());
                } else {
                    return Err(FnParseError {
                        kind: FnParseErrorKind::InvalidArgument,
                    });
                }
                buffer.clear();
                chars.next();
                last_char_was_comma = true;
            }
            '"' => {
                last_char_was_comma = false;
                buffer.push(ch);
                chars.next();
                let mut is_closed_str = false;
                while let Some(&(_, ch)) = chars.peek() {
                    buffer.push(ch);
                    chars.next();
                    if ch == '"' {
                        is_closed_str = true;
                        break;
                    }
                }
                if !is_closed_str {
                    return Err(FnParseError {
                        kind: FnParseErrorKind::UnclosedString,
                    });
                }
            }
            ch if ch.is_whitespace() => {
                buffer.push(ch);
                chars.next();
            }
            _ => {
                last_char_was_comma = false;
                buffer.push(ch);
                chars.next();
            }
        }
    }
    if !is_closed {
        return Err(FnParseError {
            kind: FnParseErrorKind::UnclosedParenthesis,
        });
    }

    // function test(a: string = "hello", b: number)
    for abstract_arg in abstract_args {
        let parts: Vec<&str> = abstract_arg.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(FnParseError {
                kind: FnParseErrorKind::InvalidArgument,
            });
        }
        if parts[0].contains(':') {
            return Err(FnParseError {
                kind: FnParseErrorKind::InvalidArgument,
            });
        }
        // a: string = "hello"
        let arg_name = parts[0].trim().to_string(); // a
        // string = "hello"
        let rest = parts[1].trim(); // string = "hello"

        let (arg_type, default_expression) = if let Some(eq_index) = rest.find('=') {
            let arg_type = rest[..eq_index].trim();
            let mut default_expr = rest[eq_index + 1..].trim();
            // 以下のような特殊な場合に対応するため
            // formatNumber(number: number, format: string = )
            if arg_type.to_lowercase() == "string" && default_expr.is_empty() {
                default_expr = "\"\"";
            }
            (arg_type.to_string(), Some(default_expr.to_string()))
        } else {
            (rest.to_string(), None)
        };
        if arg_type.contains('=') || arg_type.contains(':') || arg_type.is_empty() {
            return Err(FnParseError {
                kind: FnParseErrorKind::InvalidArgument,
            });
        }

        args.push(Arg {
            name: arg_name,
            arg_type,
            default_expression,
        });
    }

    Ok(FnParseResult {
        inner: Function { name, args },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function() {
        let input = "my_function(arg1: string = \"default\", arg2: number)";
        let result = parse(input).unwrap();
        assert_eq!(result.inner.name, "my_function");
        assert_eq!(result.inner.args.len(), 2);
        assert_eq!(result.inner.args[0].name, "arg1");
        assert_eq!(result.inner.args[0].arg_type, "string");
        assert_eq!(
            result.inner.args[0].default_expression,
            Some("\"default\"".to_string())
        );
        assert_eq!(result.inner.args[1].name, "arg2");
        assert_eq!(result.inner.args[1].arg_type, "number");
        assert_eq!(result.inner.args[1].default_expression, None);
    }

    // formatNumber(number: number, format: string = )
    #[test]
    fn test_parse_function_with_empty_string_default() {
        let input = "formatNumber(number: number, format: string = )";
        let result = parse(input).unwrap();
        assert_eq!(result.inner.name, "formatNumber");
        assert_eq!(result.inner.args.len(), 2);
        assert_eq!(result.inner.args[0].name, "number");
        assert_eq!(result.inner.args[0].arg_type, "number");
        assert_eq!(result.inner.args[0].default_expression, None);
        assert_eq!(result.inner.args[1].name, "format");
        assert_eq!(result.inner.args[1].arg_type, "string");
        assert_eq!(
            result.inner.args[1].default_expression,
            Some("\"\"".to_string())
        );
    }

    mod error_tests {
        use super::*;

        #[test]
        fn test_empty_function_name() {
            let input = "(test: string)";
            let err = parse(input).unwrap_err();
            assert_eq!(err.kind, FnParseErrorKind::EmptyFunctionName);
        }

        #[test]
        fn test_invalid_function_name_first_char() {
            let input = "1invalid(arg: string)";
            let err = parse(input).unwrap_err();
            assert_eq!(
                err.kind,
                FnParseErrorKind::InvalidFunctionName(
                    InvalidFunctionNameKind::UnexpectedFirstCharacter
                )
            );
        }

        #[test]
        fn test_invalid_function_name_character() {
            let input = "invalid-name(arg: string)";
            let err = parse(input).unwrap_err();
            assert_eq!(
                err.kind,
                FnParseErrorKind::InvalidFunctionName(InvalidFunctionNameKind::UnexpectedCharacter)
            );
        }

        #[test]
        fn test_unclosed_parenthesis() {
            let input = "func(arg: string";
            let err = parse(input).unwrap_err();
            assert_eq!(err.kind, FnParseErrorKind::UnclosedParenthesis);
        }

        #[test]
        fn test_unclosed_string() {
            let input = "func(arg: string = \"hello)";
            let err = parse(input).unwrap_err();
            assert_eq!(err.kind, FnParseErrorKind::UnclosedString);
        }

        #[test]
        fn test_function_name_contains_space() {
            let input = "my func(arg: string)";
            let err = parse(input).unwrap_err();
            assert_eq!(err.kind, FnParseErrorKind::FunctionNameContainsSpace);
        }

        #[test]
        fn test_invalid_argument() {
            let inputs = vec![
                "func1(,arg: string)",
                "func2(arg: string, )",
                "func3(arg string)",
                "func4(arg: string: number)",
                "func5(test: string a: bcd)",
                //"func6(arg: string = default = value)", // 本家の実装を後に確認
                "func7(arg: )",
            ];
            for input in inputs {
                let err = parse(input).unwrap_err();
                assert_eq!(err.kind, FnParseErrorKind::InvalidArgument);
            }
        }
    }
}
