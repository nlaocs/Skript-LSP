use super::{
    mapped_subspan, parse_context_options_with_event_classes, register_handler, request_parses,
};
use crate::nlaocs::skript_parser_addon::types::{
    ContextUpdate, ExpressionExpectedType, FunctionDeclaration, FunctionDeclarationScope,
    FunctionParameterDeclaration, FunctionReturnDeclaration, HookOutput, InvocationContext,
    MetadataEntry, ParseRequest, ParseResult, ParseResultStatus, ParserDeclaration,
    RegisteredSyntaxHandler, StructureBodyMode, StructurePayload, StructureTiming, TextRange,
};

const CLASS_SUFFIX: &str = ".StructFunction";
const HANDLER_ID: &str = "core.structure.struct-function";
const EXPRESSION_PARSER_ID: &str = "host.expression";
const FUNCTION_EVENT: &str = "ch.njol.skript.lang.function.FunctionEvent";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    payload.candidate.handler.as_deref() == Some(HANDLER_ID)
        || crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(
    context: InvocationContext,
    mut payload: StructurePayload,
    parse_results: &[ParseResult],
) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    if entering && crate::runtime::skript_at_least(2, 14).is_none() {
        return unresolved_version(context, payload);
    }
    let return_contract = if entering {
        let parsed = match parse_declaration(&payload) {
            Ok(declaration) => declaration,
            Err(reason) => return super::reject_structure(reason),
        };
        let requests = default_parse_requests(&payload, &parsed.defaults);
        let pending = super::pending_parse_requests(&requests, parse_results);
        if !pending.is_empty() {
            return request_parses(payload, pending);
        }
        if let Err(reason) = validate_default_results(&requests, &parsed.defaults, parse_results) {
            return super::reject_structure(reason);
        }
        let returns = parsed.declaration.returns.clone();
        payload
            .candidate
            .declarations
            .push(ParserDeclaration::DocumentFunction(parsed.declaration));
        Some(returns)
    } else {
        None
    };

    let mut output = super::continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Trigger,
        "function-structure",
        "core.structure.function",
    );
    if entering {
        let returns = return_contract.expect("entering Function has a return contract");
        output.effects.context_updates.extend([
            context_update(&context, "parser.event-classes", FUNCTION_EVENT),
            context_update(&context, "parser.delay-state", "false"),
            context_update(&context, "core.return-handler.available", "true"),
            context_update(
                &context,
                "core.return-handler.return-type",
                returns.class_name.as_deref().unwrap_or("void"),
            ),
            context_update(
                &context,
                "core.return-handler.single",
                if returns.single { "true" } else { "false" },
            ),
        ]);
    }
    output
}

fn unresolved_version(context: InvocationContext, payload: StructurePayload) -> HookOutput {
    let span = payload.candidate.span.clone();
    let mut output = super::continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Trigger,
        "function-structure",
        "core.structure.function",
    );
    if let Some(crate::nlaocs::skript_parser_addon::types::HookPayload::Structure(payload)) =
        output.replacement.as_mut()
    {
        super::append_metadata(payload, "semantic-state", "unresolved");
    }
    output.effects.diagnostics.push(super::structure_warning(
        "core.structure.function.unresolved-version",
        "the Skript version is unavailable, so Function signature semantics were not selected",
        span,
    ));
    output
}

fn context_update(context: &InvocationContext, key: &str, value: &str) -> ContextUpdate {
    ContextUpdate {
        syntax_context: context.syntax_context,
        key: key.to_owned(),
        value: Some(value.as_bytes().to_vec()),
    }
}

struct ParsedFunctionDeclaration {
    declaration: FunctionDeclaration,
    defaults: Vec<FunctionDefault>,
}

struct FunctionDefault {
    parameter_name: String,
    source: String,
    class_name: String,
    start: u64,
    end: u64,
}

fn parse_declaration(payload: &StructurePayload) -> Result<ParsedFunctionDeclaration, String> {
    let range = &payload.candidate.span.virtual_range;
    let start = usize::try_from(range.start)
        .map_err(|_| "Function header start does not fit usize".to_owned())?;
    let end = usize::try_from(range.end)
        .map_err(|_| "Function header end does not fit usize".to_owned())?;
    let header = payload
        .input
        .get(start..end)
        .ok_or_else(|| "Function header is not a valid UTF-8 source range".to_owned())?;
    let (scope, signature) = header
        .strip_prefix("local function ")
        .map_or_else(
            || {
                header
                    .strip_prefix("function ")
                    .map(|signature| (FunctionDeclarationScope::Global, signature))
            },
            |signature| Some((FunctionDeclarationScope::Local, signature)),
        )
        .ok_or_else(|| "invalid Function signature prefix".to_owned())?;

    let open = signature
        .find('(')
        .ok_or_else(|| "Function signature has no parameter list".to_owned())?;
    let name = signature[..open].trim();
    if !is_function_name(name) {
        return Err(format!("invalid Function name: {name}"));
    }
    let close = matching_parenthesis(signature, open)?;
    let parameter_offset = header.len() - signature.len() + open + 1;
    let (parameters, defaults) = parse_parameters_with_defaults(
        &signature[open + 1..close],
        &payload.type_options,
        // Skript switched from Functions.parseSignature to FunctionParser in 2.14.
        crate::runtime::skript_at_least(2, 14) == Some(false),
        u64::try_from(start + parameter_offset)
            .map_err(|_| "Function parameter offset does not fit u64".to_owned())?,
    )?;
    let return_suffix = &signature[close + 1..];
    let (return_source, return_syntax) = parse_return_source(return_suffix)?;
    validate_return_syntax(return_syntax)?;
    let returns = match return_source {
        Some(source) => {
            let (option, plural) =
                crate::types::match_user_type_option(source, &payload.type_options)
                    .ok_or_else(|| format!("cannot recognise Function return type: {source}"))?;
            FunctionReturnDeclaration {
                class_name: Some(option.class_name.clone()),
                single: !plural,
            }
        }
        None => FunctionReturnDeclaration {
            class_name: None,
            single: true,
        },
    };

    Ok(ParsedFunctionDeclaration {
        declaration: FunctionDeclaration {
            source: payload.input.clone(),
            span: TextRange {
                start: range.start,
                end: range.end,
            },
            scope,
            name: name.to_owned(),
            parameters,
            returns,
            metadata: vec![MetadataEntry {
                key: "function.return-syntax".to_owned(),
                value: return_syntax.to_owned(),
                owner_component_id: None,
            }],
        },
        defaults,
    })
}

#[cfg(test)]
fn parse_parameters(
    source: &str,
    type_options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
    legacy_syntax: bool,
) -> Result<Vec<FunctionParameterDeclaration>, String> {
    parse_parameters_with_defaults(source, type_options, legacy_syntax, 0)
        .map(|(parameters, _)| parameters)
}

fn parse_parameters_with_defaults(
    source: &str,
    type_options: &[crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption],
    legacy_syntax: bool,
    source_offset: u64,
) -> Result<(Vec<FunctionParameterDeclaration>, Vec<FunctionDefault>), String> {
    if source.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut parameters = Vec::new();
    let mut defaults = Vec::new();
    for (index, parameter) in split_parameter_pieces(source)?.into_iter().enumerate() {
        let (definition, default_source, default_start) = parameter.text.split_once('=').map_or(
            (parameter.text, None, None),
            |(definition, value)| {
                let trimmed = value.trim();
                let leading = value.len() - value.trim_start().len();
                (
                    definition,
                    Some(trimmed),
                    Some(parameter.start + definition.len() + 1 + leading),
                )
            },
        );
        let split = if legacy_syntax {
            definition.rsplit_once(':')
        } else {
            definition.split_once(':')
        };
        let (name, type_name) = split
            .ok_or_else(|| format!("Function parameter {} must use `name: type`", index + 1))?;
        let name = name.trim();
        let type_name = type_name.trim();
        if name.is_empty()
            || !crate::primitives::is_valid_variable_name_body(name, true)
            || !legacy_syntax
                && name
                    .chars()
                    .any(|character| matches!(character, ':' | '(' | ')' | '{' | '}' | '"' | ','))
        {
            return Err(format!("invalid Function parameter name: {name}"));
        }
        if type_name.is_empty()
            || !legacy_syntax
                && !type_name
                    .chars()
                    .all(|character| character.is_ascii_alphabetic() || character == ' ')
        {
            return Err(format!("invalid Function parameter type: {type_name}"));
        }
        if default_source.is_some_and(str::is_empty) {
            return Err(format!(
                "Function parameter {name} has an empty default value"
            ));
        }
        let (option, plural) = crate::types::match_user_type_option(type_name, type_options)
            .ok_or_else(|| format!("cannot recognise Function parameter type: {type_name}"))?;
        let name = if legacy_syntax && name.ends_with("::*") {
            let base = &name[..name.len() - "::*".len()];
            if plural {
                base.to_owned()
            } else {
                format!("{base}::1")
            }
        } else {
            name.to_owned()
        };
        let declaration = FunctionParameterDeclaration {
            name,
            class_name: option.class_name.clone(),
            single: !plural,
            default_source: default_source.map(str::to_owned),
        };
        if let (Some(default_source), Some(default_start)) = (default_source, default_start) {
            let start = source_offset
                .checked_add(
                    u64::try_from(default_start)
                        .map_err(|_| "Function default offset does not fit u64".to_owned())?,
                )
                .ok_or_else(|| "Function default offset overflowed".to_owned())?;
            let end = start
                .checked_add(
                    u64::try_from(default_source.len())
                        .map_err(|_| "Function default length does not fit u64".to_owned())?,
                )
                .ok_or_else(|| "Function default range overflowed".to_owned())?;
            defaults.push(FunctionDefault {
                parameter_name: declaration.name.clone(),
                source: default_source.to_owned(),
                class_name: declaration.class_name.clone(),
                start,
                end,
            });
        }
        parameters.push(declaration);
    }
    Ok((parameters, defaults))
}

fn default_parse_requests(
    payload: &StructurePayload,
    defaults: &[FunctionDefault],
) -> Vec<ParseRequest> {
    defaults
        .iter()
        .enumerate()
        .map(|(index, default)| ParseRequest {
            request_id: index as u64,
            parser_id: EXPRESSION_PARSER_ID.to_owned(),
            input: default.source.clone(),
            expected_types: vec![default_expected_type(default)],
            span: mapped_subspan(&payload.candidate.span, default.start, default.end),
            options: parse_context_options_with_event_classes(&payload.context, &[FUNCTION_EVENT]),
        })
        .collect()
}

fn default_expected_type(default: &FunctionDefault) -> ExpressionExpectedType {
    ExpressionExpectedType {
        class_name: default.class_name.clone(),
        plural: false,
    }
}

fn validate_default_results(
    requests: &[ParseRequest],
    defaults: &[FunctionDefault],
    results: &[ParseResult],
) -> Result<(), String> {
    for (request, default) in requests.iter().zip(defaults) {
        let Some(result) = results.iter().find(|result| {
            result.request_id == request.request_id && result.parser_id == request.parser_id
        }) else {
            return Err("Function default Expression parse result is missing".to_owned());
        };
        if result.status != ParseResultStatus::Success {
            return Err(format!(
                "default value for Function parameter `{}` is not a valid {} Expression",
                default.parameter_name, request.expected_types[0].class_name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn split_parameters(source: &str) -> Result<Vec<&str>, String> {
    split_parameter_pieces(source)
        .map(|pieces| pieces.into_iter().map(|piece| piece.text).collect())
}

struct ParameterPiece<'a> {
    text: &'a str,
    start: usize,
}

fn split_parameter_pieces(source: &str) -> Result<Vec<ParameterPiece<'_>>, String> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut parentheses = 0_u32;
    let mut braces = 0_u32;
    let mut quoted = false;
    let mut characters = source.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' if quoted && characters.peek().is_some_and(|(_, next)| *next == '"') => {
                characters.next();
            }
            '"' => quoted = !quoted,
            '(' if !quoted => parentheses += 1,
            ')' if !quoted => {
                parentheses = parentheses
                    .checked_sub(1)
                    .ok_or_else(|| "unmatched `)` in Function parameters".to_owned())?;
            }
            '{' if !quoted => braces += 1,
            '}' if !quoted => {
                braces = braces
                    .checked_sub(1)
                    .ok_or_else(|| "unmatched `}` in Function parameters".to_owned())?;
            }
            ',' if !quoted && parentheses == 0 && braces == 0 => {
                let raw = &source[start..index];
                let piece = raw.trim();
                if piece.is_empty() {
                    return Err("Function parameter definition is empty".to_owned());
                }
                pieces.push(ParameterPiece {
                    text: piece,
                    start: start + raw.len() - raw.trim_start().len(),
                });
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if quoted || parentheses != 0 || braces != 0 {
        return Err("unclosed text, variable, or parentheses in Function parameters".to_owned());
    }
    let raw = &source[start..];
    let tail = raw.trim();
    if tail.is_empty() {
        return Err("Function parameter definition is empty".to_owned());
    }
    pieces.push(ParameterPiece {
        text: tail,
        start: start + raw.len() - raw.trim_start().len(),
    });
    Ok(pieces)
}

fn matching_parenthesis(source: &str, open: usize) -> Result<usize, String> {
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut characters = source[open..].char_indices().peekable();
    while let Some((offset, character)) = characters.next() {
        match character {
            '"' if quoted && characters.peek().is_some_and(|(_, next)| *next == '"') => {
                characters.next();
            }
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + offset);
                }
            }
            _ => {}
        }
    }
    Err("Function signature has an unclosed parameter list".to_owned())
}

fn parse_return_source(source: &str) -> Result<(Option<&str>, &'static str), String> {
    let source = source.trim();
    if source.is_empty() {
        return Ok((None, "none"));
    }
    for (prefix, name) in [
        ("->", "arrow"),
        ("::", "double-colon"),
        ("returns ", "returns"),
    ] {
        if let Some(value) = source.strip_prefix(prefix) {
            let value = value.trim();
            return (!value.is_empty())
                .then_some((Some(value), name))
                .ok_or_else(|| "Function return type is empty".to_owned());
        }
    }
    Err("invalid Function return declaration".to_owned())
}

fn validate_return_syntax(syntax: &str) -> Result<(), String> {
    if syntax == "none" || syntax == "double-colon" {
        return Ok(());
    }
    // Upstream StructFunction accepted only `::` through 2.7. `returns` was added in
    // 2.8, while the arrow spelling was added to SIGNATURE_PATTERN in Skript 2.14.
    if syntax == "returns" && crate::runtime::skript_at_least(2, 8) == Some(true) {
        return Ok(());
    }
    if syntax == "arrow" && crate::runtime::skript_at_least(2, 14) == Some(true) {
        return Ok(());
    }
    Err(format!(
        "Function return syntax `{syntax}` is not supported by this Skript version"
    ))
}

fn is_function_name(name: &str) -> bool {
    let mut characters = name.chars();
    let allow_leading_underscore = crate::runtime::skript_at_least(2, 12) == Some(true);
    characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || allow_leading_underscore && first == '_')
        && characters.all(|character| character.is_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::{
        FUNCTION_EVENT, FunctionDefault, default_expected_type, parse_parameters,
        parse_parameters_with_defaults, parse_return_source, split_parameters,
    };
    use crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption;

    fn number_type() -> ExpressionTypeOption {
        ExpressionTypeOption {
            source_record: None,
            definition_id: "type:number".to_owned(),
            registration_id: "type:number:0".to_owned(),
            addon_name: "fixture".to_owned(),
            addon_version: "1.0.0".to_owned(),
            code_name: "number".to_owned(),
            class_name: "java.lang.Number".to_owned(),
            parser_class: None,
            type_parse_order: 0,
            before: Vec::new(),
            after: Vec::new(),
            singular: "number".to_owned(),
            plural: "numbers".to_owned(),
            user_input_patterns: vec!["num(ber)?s?".to_owned()],
            has_parser: true,
            parse_contexts: vec!["DEFAULT".to_owned(), "COMMAND".to_owned()],
            has_supplier: false,
            default_expression: None,
        }
    }

    #[test]
    fn parameters_preserve_plurality_defaults_and_nested_commas() {
        let source = "values: numbers = list(1, 2), scale: number = 1";
        let parameters = parse_parameters(source, &[number_type()], false).unwrap();
        assert_eq!(parameters.len(), 2);
        assert!(!parameters[0].single);
        assert_eq!(parameters[0].default_source.as_deref(), Some("list(1, 2)"));
        assert!(parameters[1].single);
        assert_eq!(parameters[1].default_source.as_deref(), Some("1"));
    }

    #[test]
    fn default_ranges_point_at_the_exact_expression_source() {
        let source = "values: numbers = list(1, 2), scale: number = 1";
        let (_, defaults) =
            parse_parameters_with_defaults(source, &[number_type()], false, 100).unwrap();
        assert_eq!(defaults.len(), 2);
        for default in defaults {
            let start = usize::try_from(default.start - 100).unwrap();
            let end = usize::try_from(default.end - 100).unwrap();
            assert_eq!(&source[start..end], default.source);
        }
        assert_eq!(FUNCTION_EVENT, "ch.njol.skript.lang.function.FunctionEvent");
    }

    #[test]
    fn function_defaults_use_the_component_type_and_single_parse() {
        let default = FunctionDefault {
            parameter_name: "values".to_owned(),
            source: "list(1, 2)".to_owned(),
            class_name: "java.lang.Number".to_owned(),
            start: 0,
            end: 9,
        };

        let expected = default_expected_type(&default);
        assert_eq!(expected.class_name, "java.lang.Number");
        assert!(!expected.plural);
    }

    #[test]
    fn parameter_split_ignores_commas_in_text_and_variables() {
        assert_eq!(
            split_parameters("a: number = \"1, 2\", b: number = {value::1,2}").unwrap(),
            ["a: number = \"1, 2\"", "b: number = {value::1,2}"]
        );
    }

    #[test]
    fn legacy_parameter_syntax_accepts_the_old_list_variable_name() {
        let parameters = parse_parameters("values::*: numbers", &[number_type()], true).unwrap();
        assert_eq!(parameters[0].name, "values");
        assert!(!parameters[0].single);
    }

    #[test]
    fn return_spelling_is_retained_for_version_policy_validation() {
        assert_eq!(
            parse_return_source(" returns numbers").unwrap(),
            (Some("numbers"), "returns")
        );
        assert_eq!(
            parse_return_source(" -> number").unwrap(),
            (Some("number"), "arrow")
        );
        assert_eq!(
            parse_return_source(" :: number").unwrap(),
            (Some("number"), "double-colon")
        );
    }
}
