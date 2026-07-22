use skripthub::api::types::{AbstractAddonSyntaxListEntry, SyntaxType};
use syntax_pattern_parser::function::{FnParseError, FnParseErrorKind, InvalidFunctionNameKind};
use syntax_pattern_parser::syntax::{ParseError, ParseErrorKind};
use syntaxes::Syntaxes;

fn dump_patterns(label: &str, list: Vec<AbstractAddonSyntaxListEntry>) {
    println!("{}:", label);
    for s in list {
        let patterns = s.syntax_pattern.lines();
        for p in patterns {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            match s.syntax_type {
                SyntaxType::Function => {
                    let parsed = syntax_pattern_parser::function::parse(p);
                    match parsed {
                        Ok(_) => {}
                        Err(_) => {
                            println!("    https://skripthub.net/docs/?id={} (func): {}", s.id, p);
                        }
                    }
                }
                _ => {
                    let parsed = syntax_pattern_parser::syntax::parse(p);
                    match parsed {
                        Ok(_) => {}
                        Err(_) => {
                            println!("    https://skripthub.net/docs/?id={}: {}", s.id, p);
                        }
                    }
                }
            }
        }
    }
    println!();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let syntaxes = Syntaxes::initialize()?;
    let errors_syntaxes = syntaxes.1;

    let mut unclosed_parenthesis: Vec<_> = Vec::new();
    let mut unexpected_closing_parenthesis: Vec<_> = Vec::new();
    let mut unclosed_bracket: Vec<_> = Vec::new();
    let mut unexpected_closing_bracket: Vec<_> = Vec::new();
    let mut unclosed_type_delimiter: Vec<_> = Vec::new();
    let mut unclosed_regex_delimiter: Vec<_> = Vec::new();
    let mut unexpected_closing_regex_delimiter: Vec<_> = Vec::new();
    let mut incorrect_time_state: Vec<_> = Vec::new();
    let mut invalid_parse_mark: Vec<_> = Vec::new();

    let mut invalid_function_name_unexpected_first_character: Vec<_> = Vec::new();
    let mut invalid_function_name_unexpected_character: Vec<_> = Vec::new();
    let mut empty_function_name: Vec<_> = Vec::new();
    let mut invalid_argument: Vec<_> = Vec::new();
    let mut fn_unclosed_parenthesis: Vec<_> = Vec::new();
    let mut fn_unclosed_string: Vec<_> = Vec::new();
    let mut function_name_contains_space: Vec<_> = Vec::new();

    let mut unknown: Vec<_> = Vec::new();

    for (syntax, err) in errors_syntaxes {
        if let Some(parse_err) = err.downcast_ref::<ParseError>() {
            let kind = parse_err.kind;
            match kind {
                ParseErrorKind::UnclosedParenthesis => {
                    unclosed_parenthesis.push(syntax);
                }
                ParseErrorKind::UnexpectedClosingParenthesis => {
                    unexpected_closing_parenthesis.push(syntax);
                }
                ParseErrorKind::UnclosedBracket => {
                    unclosed_bracket.push(syntax);
                }
                ParseErrorKind::UnexpectedClosingBracket => {
                    unexpected_closing_bracket.push(syntax);
                }
                ParseErrorKind::UnclosedTypeDelimiter => {
                    unclosed_type_delimiter.push(syntax);
                }
                ParseErrorKind::UnclosedRegexDelimiter => {
                    unclosed_regex_delimiter.push(syntax);
                }
                ParseErrorKind::UnexpectedClosingRegexDelimiter => {
                    unexpected_closing_regex_delimiter.push(syntax);
                }
                ParseErrorKind::IncorrectTimeState => {
                    incorrect_time_state.push(syntax);
                }
                ParseErrorKind::InvalidParseMark => {
                    invalid_parse_mark.push(syntax);
                }
            }
        } else if let Some(fn_parse_err) = err.downcast_ref::<FnParseError>() {
            let kind = &fn_parse_err.kind;
            match kind {
                FnParseErrorKind::InvalidFunctionName(kind) => match kind {
                    InvalidFunctionNameKind::UnexpectedFirstCharacter => {
                        invalid_function_name_unexpected_first_character.push(syntax);
                    }
                    InvalidFunctionNameKind::UnexpectedCharacter => {
                        invalid_function_name_unexpected_character.push(syntax);
                    }
                },
                FnParseErrorKind::EmptyFunctionName => {
                    empty_function_name.push(syntax);
                }
                FnParseErrorKind::InvalidArgument => {
                    invalid_argument.push(syntax);
                }
                FnParseErrorKind::UnclosedParenthesis => {
                    fn_unclosed_parenthesis.push(syntax);
                }
                FnParseErrorKind::UnclosedString => {
                    fn_unclosed_string.push(syntax);
                }
                FnParseErrorKind::FunctionNameContainsSpace => {
                    function_name_contains_space.push(syntax);
                }
            }
        } else {
            unknown.push(syntax);
        }
    }

    dump_patterns("unclosed_parenthesis", unclosed_parenthesis);
    dump_patterns(
        "unexpected_closing_parenthesis",
        unexpected_closing_parenthesis,
    );
    dump_patterns("unclosed_bracket", unclosed_bracket);
    dump_patterns("unexpected_closing_bracket", unexpected_closing_bracket);
    dump_patterns("unclosed_type_delimiter", unclosed_type_delimiter);
    dump_patterns("unclosed_regex_delimiter", unclosed_regex_delimiter);
    dump_patterns(
        "unexpected_closing_regex_delimiter",
        unexpected_closing_regex_delimiter,
    );
    dump_patterns("incorrect_time_state", incorrect_time_state);
    dump_patterns("invalid_parse_mark", invalid_parse_mark);

    dump_patterns(
        "invalid_function_name_unexpected_first_character",
        invalid_function_name_unexpected_first_character,
    );
    dump_patterns(
        "invalid_function_name_unexpected_character",
        invalid_function_name_unexpected_character,
    );
    dump_patterns("empty_function_name", empty_function_name);
    dump_patterns("invalid_argument", invalid_argument);
    dump_patterns("fn_unclosed_parenthesis", fn_unclosed_parenthesis);
    dump_patterns("fn_unclosed_string", fn_unclosed_string);

    dump_patterns("unknown", unknown);

    Ok(())
}
