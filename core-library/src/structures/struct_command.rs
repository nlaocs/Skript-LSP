use super::{mapped_subspan, parse_context_options, register_handler, request_parses};
#[cfg(target_arch = "wasm32")]
use crate::nlaocs::skript_parser_addon::state_store;
use crate::nlaocs::skript_parser_addon::types::{
    ContextUpdate, ExpressionExpectedType, ExpressionTypeOption, HookOutput, InvocationContext,
    MetadataEntry, ParseRequest, ParseResult, ParseResultStatus, RegisteredSyntaxHandler,
    StructureBodyMode, StructurePayload, StructureTiming,
};
#[cfg(target_arch = "wasm32")]
use crate::nlaocs::skript_parser_addon::types::{
    StateEncoding, StateNamespaceVisibility, StateScope, StateValue,
};

const CLASS_SUFFIX: &str = ".StructCommand";
const HANDLER_ID: &str = "core.structure.struct-command";
const COMMAND_EVENT: &str = "ch.njol.skript.command.ScriptCommandEvent";
const EXPRESSION_PARSER_ID: &str = "host.expression";
const PARSE_MODE: &str = "parse.mode";
const PARSE_CONTEXT: &str = "context.value.parser.parse-context";
#[cfg(target_arch = "wasm32")]
const COMMAND_NAMESPACE: &str = "commands";
#[cfg(target_arch = "wasm32")]
const COMMAND_SCHEMA: &str = "nlaocs.core-library.commands";

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
    let command = if entering {
        let (header_start, header) = header_source(&payload);
        match parse_command_header(header, &payload.type_options) {
            Ok(command) => {
                let requests = default_parse_requests(
                    &payload.candidate.span,
                    &payload.context,
                    &command,
                    header_start,
                );
                let pending = super::pending_parse_requests(&requests, parse_results);
                if !pending.is_empty() {
                    return request_parses(payload, pending);
                }
                if let Err(reason) = validate_default_results(&requests, parse_results) {
                    return super::reject_structure(reason);
                }
                if let Err(reason) = register_command_name(&command.name, &context.document_id) {
                    return super::reject_structure(reason);
                }
                payload.candidate.metadata.extend(command.metadata());
                Some(command)
            }
            Err(reason) => return super::reject_structure(reason),
        }
    } else {
        None
    };

    let mut output = super::continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Entries,
        "command-structure",
        "core.structure.command",
    );
    append_entry_warnings(&mut output);
    if let Some(command) = command {
        output.effects.context_updates.extend([
            ContextUpdate {
                syntax_context: context.syntax_context,
                key: "parser.event-classes".to_owned(),
                value: Some(COMMAND_EVENT.as_bytes().to_vec()),
            },
            ContextUpdate {
                syntax_context: context.syntax_context,
                key: "core.command.name".to_owned(),
                value: Some(command.name.as_bytes().to_vec()),
            },
            ContextUpdate {
                syntax_context: context.syntax_context,
                key: "parser.delay-state".to_owned(),
                value: Some(b"false".to_vec()),
            },
        ]);
        output
            .effects
            .context_updates
            .extend(command.context_updates(&context));
    }
    output
}

#[cfg(target_arch = "wasm32")]
fn register_command_name(name: &str, document_id: &str) -> Result<(), String> {
    let key = name.to_ascii_lowercase();
    let existing = state_store::get(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        COMMAND_NAMESPACE,
        &key,
    )
    .map_err(|error| {
        format!(
            "failed to inspect script command declarations: {}",
            error.message
        )
    })?;
    if existing.is_some() {
        return Err(format!(
            "a command with the name /{name} is already defined"
        ));
    }
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        COMMAND_NAMESPACE,
        &key,
        &StateValue {
            schema_id: COMMAND_SCHEMA.to_owned(),
            encoding: StateEncoding::Raw,
            bytes: document_id.as_bytes().to_vec(),
        },
    )
    .map_err(|error| {
        format!(
            "failed to record script command declaration: {}",
            error.message
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn register_command_name(_name: &str, _document_id: &str) -> Result<(), String> {
    Ok(())
}

fn append_entry_warnings(output: &mut HookOutput) {
    let Some(payload) = output
        .replacement
        .as_ref()
        .and_then(|replacement| match replacement {
            crate::nlaocs::skript_parser_addon::types::HookPayload::Structure(payload) => {
                Some(payload)
            }
            _ => None,
        })
    else {
        return;
    };
    let explicit = |key: &str| {
        payload
            .candidate
            .entries
            .iter()
            .find(|entry| entry.key.eq_ignore_ascii_case(key) && !entry.defaulted)
    };
    let permission = explicit("permission");
    if let Some(message) = explicit("permission message")
        && permission.is_none_or(|entry| empty_entry_value(&entry.source))
    {
        output.effects.diagnostics.push(super::structure_warning(
            "core.struct-command.permission-message-without-permission",
            "the command has a permission message but no permission",
            message.span.clone(),
        ));
    }

    if explicit("cooldown").is_some() {
        return;
    }
    for (key, code, message) in [
        (
            "cooldown message",
            "core.struct-command.cooldown-message-without-cooldown",
            "the command has a cooldown message but no cooldown",
        ),
        (
            "cooldown storage",
            "core.struct-command.cooldown-storage-without-cooldown",
            "the command has cooldown storage but no cooldown",
        ),
    ] {
        if let Some(entry) = explicit(key) {
            output.effects.diagnostics.push(super::structure_warning(
                code,
                message,
                entry.span.clone(),
            ));
        }
    }
    // StructCommand only warns for an explicitly empty bypass. A non-empty
    // bypass without a cooldown is accepted by Skript 2.15.4.
    if let Some(entry) = explicit("cooldown bypass")
        && empty_entry_value(&entry.source)
    {
        output.effects.diagnostics.push(super::structure_warning(
            "core.struct-command.empty-cooldown-bypass-without-cooldown",
            "the command has an empty cooldown bypass but no cooldown",
            entry.span.clone(),
        ));
    }
}

fn empty_entry_value(source: &str) -> bool {
    let source = source.trim();
    source.is_empty() || source == "\"\""
}

fn header_source(payload: &StructurePayload) -> (u64, &str) {
    let range = &payload.candidate.span.virtual_range;
    usize::try_from(range.start)
        .ok()
        .zip(usize::try_from(range.end).ok())
        .and_then(|(start, end)| {
            payload
                .input
                .get(start..end)
                .map(|source| (range.start, source))
        })
        .unwrap_or((0, &payload.input))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandHeader {
    name: String,
    arguments: Vec<CommandArgument>,
    defaults: Vec<CommandDefault>,
}

impl CommandHeader {
    fn metadata(&self) -> Vec<MetadataEntry> {
        let mut metadata = vec![
            entry("command.name", &self.name),
            entry("command.argument-count", &self.arguments.len().to_string()),
        ];
        for (index, argument) in self.arguments.iter().enumerate() {
            let prefix = format!("command.argument.{index}");
            if let Some(name) = &argument.name {
                metadata.push(entry(&format!("{prefix}.name"), name));
            }
            metadata.extend([
                entry(&format!("{prefix}.type"), &argument.type_name),
                entry(&format!("{prefix}.class"), &argument.class_name),
                entry(&format!("{prefix}.single"), &argument.single.to_string()),
                entry(
                    &format!("{prefix}.optional"),
                    &argument.optional.to_string(),
                ),
                entry(
                    &format!("{prefix}.parse-context-state"),
                    if argument.parse_context_known {
                        "resolved"
                    } else {
                        "unresolved"
                    },
                ),
            ]);
            if let Some(default) = &argument.default_source {
                metadata.push(entry(&format!("{prefix}.default"), default));
            }
        }
        metadata
    }

    fn context_updates(&self, context: &InvocationContext) -> Vec<ContextUpdate> {
        let mut updates = vec![ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.command.argument-count".to_owned(),
            value: Some(self.arguments.len().to_string().into_bytes()),
        }];
        for (index, argument) in self.arguments.iter().enumerate() {
            let prefix = format!("core.command.argument.{index}");
            for (suffix, value) in [
                ("class", argument.class_name.as_str()),
                ("single", if argument.single { "true" } else { "false" }),
                ("optional", if argument.optional { "true" } else { "false" }),
            ] {
                updates.push(ContextUpdate {
                    syntax_context: context.syntax_context,
                    key: format!("{prefix}.{suffix}"),
                    value: Some(value.as_bytes().to_vec()),
                });
            }
            if let Some(name) = &argument.name {
                updates.push(ContextUpdate {
                    syntax_context: context.syntax_context,
                    key: format!("{prefix}.name"),
                    value: Some(name.as_bytes().to_vec()),
                });
            }
        }
        updates
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandArgument {
    name: Option<String>,
    type_name: String,
    class_name: String,
    single: bool,
    optional: bool,
    default_source: Option<String>,
    parse_context_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultParseMode {
    All,
    ExpressionsOnly,
    LiteralsOnly,
}

impl DefaultParseMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ExpressionsOnly => "expressions-only",
            Self::LiteralsOnly => "literals-only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultParseContext {
    Command,
    Default,
}

impl DefaultParseContext {
    fn as_str(self) -> &'static str {
        match self {
            Self::Command => "COMMAND",
            Self::Default => "DEFAULT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandDefault {
    argument_index: usize,
    source: String,
    input: String,
    mode: DefaultParseMode,
    context: DefaultParseContext,
    input_start: usize,
    input_end: usize,
}

/// Mirrors `StructCommand.load` and `Argument.newInstance` from Skript.
/// Java balances optional brackets before it walks `<name: type = default>`
/// placeholders. A default starts an implicit optional suffix, making every
/// later argument optional even when the source does not contain `[...]`.
fn parse_command_header(
    source: &str,
    type_options: &[ExpressionTypeOption],
) -> Result<CommandHeader, String> {
    validate_optional_brackets(source)?;
    let leading = source.len() - source.trim_start().len();
    let source = source.trim();
    let Some(keyword_end) = source.find(char::is_whitespace) else {
        return Err("invalid command structure pattern".to_owned());
    };
    if !source[..keyword_end].eq_ignore_ascii_case("command") {
        return Err("invalid command structure pattern".to_owned());
    }
    let remainder = source[keyword_end..].trim_start();
    let remainder = remainder.strip_prefix('/').unwrap_or(remainder);
    let name_end = remainder
        .find(char::is_whitespace)
        .unwrap_or(remainder.len());
    let name = &remainder[..name_end];
    if name.is_empty() {
        return Err("command name is empty".to_owned());
    }
    let raw_arguments = &remainder[name_end..];
    let arguments = raw_arguments.trim_start();
    let arguments_offset = leading + source.len() - arguments.len();
    let (arguments, defaults) = parse_arguments(arguments, type_options, arguments_offset)?;
    Ok(CommandHeader {
        name: name.to_ascii_lowercase(),
        arguments,
        defaults,
    })
}

fn validate_optional_brackets(source: &str) -> Result<(), String> {
    let mut level = 0usize;
    for character in source.chars() {
        match character {
            '[' => level += 1,
            ']' if level == 0 => return Err("invalid placement of [optional brackets]".to_owned()),
            ']' => level -= 1,
            _ => {}
        }
    }
    (level == 0)
        .then_some(())
        .ok_or_else(|| "invalid amount of [optional brackets]".to_owned())
}

fn parse_arguments(
    source: &str,
    type_options: &[ExpressionTypeOption],
    source_offset: usize,
) -> Result<(Vec<CommandArgument>, Vec<CommandDefault>), String> {
    let mut arguments = Vec::new();
    let mut defaults = Vec::new();
    let mut cursor = 0usize;
    let mut optional_depth = 0usize;
    let mut implicit_optional = false;
    while cursor < source.len() {
        let character = source[cursor..]
            .chars()
            .next()
            .expect("cursor is on a character boundary");
        match character {
            '[' => optional_depth += 1,
            ']' => optional_depth = optional_depth.saturating_sub(1),
            '<' => {
                let argument_index = arguments.len();
                let end = placeholder_end(source, cursor + 1)
                    .ok_or_else(|| "command argument has an unclosed `<`".to_owned())?;
                let (mut argument, default) = parse_argument(
                    &source[cursor + 1..end],
                    type_options,
                    optional_depth > 0 || implicit_optional,
                    source_offset + cursor + 1,
                    argument_index,
                )?;
                if argument.default_source.is_some() && optional_depth == 0 {
                    implicit_optional = true;
                    argument.optional = true;
                }
                arguments.push(argument);
                if let Some(default) = default {
                    defaults.push(default);
                }
                cursor = end + 1;
                continue;
            }
            _ => {}
        }
        cursor += character.len_utf8();
    }
    Ok((arguments, defaults))
}

fn placeholder_end(source: &str, start: usize) -> Option<usize> {
    let mut quoted = false;
    let mut braces = 0usize;
    let mut parentheses = 0usize;
    let mut characters = source[start..].char_indices().peekable();
    while let Some((offset, character)) = characters.next() {
        match character {
            '"' if quoted && characters.peek().is_some_and(|(_, next)| *next == '"') => {
                characters.next();
            }
            '"' => quoted = !quoted,
            '{' if !quoted => braces += 1,
            '}' if !quoted && braces > 0 => braces -= 1,
            '(' if !quoted => parentheses += 1,
            ')' if !quoted && parentheses > 0 => parentheses -= 1,
            '>' if !quoted && braces == 0 && parentheses == 0 => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn parse_argument(
    source: &str,
    type_options: &[ExpressionTypeOption],
    optional: bool,
    source_offset: usize,
    argument_index: usize,
) -> Result<(CommandArgument, Option<CommandDefault>), String> {
    let (definition, default_source, default_start) = split_top_level_at(source, '=').map_or(
        (source, None, None),
        |(definition, value, separator)| {
            let trimmed = value.trim();
            let leading = value.len() - value.trim_start().len();
            (
                definition,
                Some(trimmed),
                Some(separator + '='.len_utf8() + leading),
            )
        },
    );
    if default_source.is_some_and(str::is_empty) {
        return Err("command argument has an empty default value".to_owned());
    }
    let (name, type_name) = split_top_level(definition, ':')
        .map_or((None, definition.trim()), |(name, type_name)| {
            (Some(name.trim()), type_name.trim())
        });
    if name.is_some_and(|name| {
        name.is_empty() || !crate::primitives::is_valid_variable_name_body(name, false)
    }) {
        return Err("an argument's name must be a valid non-list variable name".to_owned());
    }
    if type_name.is_empty() {
        return Err("command argument type is empty".to_owned());
    }
    let (type_option, plural) = crate::types::match_user_type_option(type_name, type_options)
        .ok_or_else(|| format!("unknown command argument type `{type_name}`"))?;
    if !type_option.has_parser {
        return Err(format!(
            "can't use {} as argument of a command: the type has no parser",
            type_option.code_name
        ));
    }
    let parse_context_known = !type_option.parse_contexts.is_empty();
    if parse_context_known
        && !type_option
            .parse_contexts
            .iter()
            .any(|context| context.eq_ignore_ascii_case("COMMAND"))
    {
        return Err(format!(
            "can't use {} as argument of a command: its parser does not accept COMMAND",
            type_option.code_name
        ));
    }
    let argument = CommandArgument {
        name: name.map(str::to_owned),
        type_name: type_option.code_name.clone(),
        class_name: type_option.class_name.clone(),
        single: !plural,
        optional: optional || default_source.is_some(),
        default_source: default_source.map(str::to_owned),
        parse_context_known,
    };
    let default = default_source.and_then(|default_source| {
        default_start.and_then(|start| {
            command_default(
                argument_index,
                default_source,
                source_offset + start,
                &type_option.class_name,
            )
        })
    });
    Ok((argument, default))
}

fn split_top_level(source: &str, separator: char) -> Option<(&str, &str)> {
    split_top_level_at(source, separator).map(|(left, right, _)| (left, right))
}

fn split_top_level_at(source: &str, separator: char) -> Option<(&str, &str, usize)> {
    let mut quoted = false;
    let mut braces = 0usize;
    let mut parentheses = 0usize;
    let mut characters = source.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' if quoted && characters.peek().is_some_and(|(_, next)| *next == '"') => {
                characters.next();
            }
            '"' => quoted = !quoted,
            '{' if !quoted => braces += 1,
            '}' if !quoted && braces > 0 => braces -= 1,
            '(' if !quoted => parentheses += 1,
            ')' if !quoted && parentheses > 0 => parentheses -= 1,
            value if value == separator && !quoted && braces == 0 && parentheses == 0 => {
                return Some((&source[..index], &source[index + value.len_utf8()..], index));
            }
            _ => {}
        }
    }
    None
}

fn command_default(
    argument_index: usize,
    source: &str,
    start: usize,
    class_name: &str,
) -> Option<CommandDefault> {
    // Argument.newInstance accepts an unquoted String default as a
    // SimpleLiteral. It therefore needs no recursive host parse request.
    if class_name == "java.lang.String"
        && !source.starts_with('%')
        && !(source.starts_with('"') && source.ends_with('"'))
    {
        return None;
    }
    if source.len() >= 2 && source.starts_with('%') && source.ends_with('%') {
        return Some(CommandDefault {
            argument_index,
            source: source.to_owned(),
            input: source[1..source.len() - 1].to_owned(),
            mode: DefaultParseMode::ExpressionsOnly,
            context: DefaultParseContext::Command,
            input_start: start + 1,
            input_end: start + source.len() - 1,
        });
    }
    if class_name == "java.lang.String"
        && source.len() >= 2
        && source.starts_with('"')
        && source.ends_with('"')
    {
        return Some(CommandDefault {
            argument_index,
            source: source.to_owned(),
            input: source.to_owned(),
            mode: DefaultParseMode::All,
            context: DefaultParseContext::Default,
            input_start: start,
            input_end: start + source.len(),
        });
    }
    Some(CommandDefault {
        argument_index,
        source: source.to_owned(),
        input: source.to_owned(),
        mode: DefaultParseMode::LiteralsOnly,
        context: DefaultParseContext::Default,
        input_start: start,
        input_end: start + source.len(),
    })
}

fn default_parse_requests(
    parent_span: &crate::nlaocs::skript_parser_addon::types::MappedSpan,
    parse_context: &crate::nlaocs::skript_parser_addon::types::ParseContext,
    command: &CommandHeader,
    header_start: u64,
) -> Vec<ParseRequest> {
    command
        .defaults
        .iter()
        .enumerate()
        .map(|(request_id, default)| {
            let mut options = parse_context_options(parse_context);
            // StructCommand calls Argument.newInstance while ScriptCommandEvent
            // is the active parser event. Keep that event context even when the
            // command structure itself is nested in another parse context.
            options.push(entry("context.event-classes", COMMAND_EVENT));
            options.push(entry(PARSE_CONTEXT, default.context.as_str()));
            options.push(entry(PARSE_MODE, default.mode.as_str()));
            let start = header_start.saturating_add(default.input_start as u64);
            let end = header_start.saturating_add(default.input_end as u64);
            let argument = &command.arguments[default.argument_index];
            ParseRequest {
                request_id: request_id as u64,
                parser_id: EXPRESSION_PARSER_ID.to_owned(),
                input: default.input.clone(),
                expected_types: vec![ExpressionExpectedType {
                    class_name: argument.class_name.clone(),
                    // Argument.newInstance passes only the argument class to
                    // parseExpression; the command argument's `single` flag
                    // does not constrain the default value parser.
                    plural: true,
                }],
                span: mapped_subspan(parent_span, start, end),
                options,
            }
        })
        .collect()
}

fn validate_default_results(
    requests: &[ParseRequest],
    results: &[ParseResult],
) -> Result<(), String> {
    if requests.len() != results.len() {
        return Err("command default parse results do not match the requested defaults".to_owned());
    }
    for request in requests {
        let Some(result) = results.iter().find(|result| {
            result.request_id == request.request_id && result.parser_id == request.parser_id
        }) else {
            return Err("command default Expression parse result is missing".to_owned());
        };
        if result.status != ParseResultStatus::Success {
            let expected_class = request
                .expected_types
                .first()
                .map_or("the expected type", |expected| expected.class_name.as_str());
            return Err(format!(
                "default value `{}` is not a valid {} Expression",
                request.input, expected_class
            ));
        }
        if result.roots.is_empty() {
            return Err("a successful command default parse result has no root".to_owned());
        }
    }
    Ok(())
}

fn entry(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COMMAND_EVENT, DefaultParseContext, DefaultParseMode, default_parse_requests,
        parse_command_header, validate_default_results,
    };
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionTypeOption, InvocationContext, MappedSpan, OriginKind, ParseContext,
        ParseContextValue, ParseRequest, ParseResult, ParseResultStatus, SourceOrigin, TextRange,
    };

    fn option(
        code_name: &str,
        singular: &str,
        plural: &str,
        class_name: &str,
        contexts: &[&str],
    ) -> ExpressionTypeOption {
        ExpressionTypeOption {
            source_record: None,
            definition_id: format!("type:{code_name}"),
            registration_id: format!("type:{code_name}:0"),
            addon_name: "fixture".to_owned(),
            addon_version: "1.0.0".to_owned(),
            code_name: code_name.to_owned(),
            class_name: class_name.to_owned(),
            parser_class: None,
            type_parse_order: 0,
            before: Vec::new(),
            after: Vec::new(),
            singular: singular.to_owned(),
            plural: plural.to_owned(),
            user_input_patterns: vec![format!("(?:{singular}|{plural})")],
            has_parser: true,
            parse_contexts: contexts.iter().map(|value| (*value).to_owned()).collect(),
            has_supplier: false,
        }
    }

    fn options() -> Vec<ExpressionTypeOption> {
        vec![
            option(
                "string",
                "string",
                "strings",
                "java.lang.String",
                &["DEFAULT", "COMMAND"],
            ),
            option(
                "player",
                "player",
                "players",
                "org.bukkit.entity.Player",
                &["COMMAND"],
            ),
        ]
    }

    fn source_span(start: u64, end: u64) -> MappedSpan {
        let range = TextRange { start, end };
        MappedSpan {
            virtual_range: range,
            origins: vec![SourceOrigin {
                original_range: range,
                kind: OriginKind::Exact,
                expansion: None,
            }],
        }
    }

    fn parse_context() -> ParseContext {
        ParseContext {
            syntax_context: 9,
            event_classes: vec!["fixture.OuterEvent".to_owned()],
            section_stack: Vec::new(),
            values: vec![ParseContextValue {
                key: "outer-value".to_owned(),
                value: "kept".to_owned(),
            }],
        }
    }

    fn option_value<'a>(request: &'a ParseRequest, key: &str) -> &'a str {
        request
            .options
            .iter()
            .rfind(|option| option.key == key)
            .map(|option| option.value.as_str())
            .expect("expected request option")
    }

    #[test]
    fn parses_named_plural_optional_and_default_arguments() {
        let command = parse_command_header(
            "COMMAND /broadcast <message: string> [<targets: players = all players>]",
            &options(),
        )
        .unwrap();
        assert_eq!(command.name, "broadcast");
        assert_eq!(command.arguments.len(), 2);
        assert_eq!(command.arguments[0].name.as_deref(), Some("message"));
        assert!(command.arguments[0].single);
        assert!(!command.arguments[0].optional);
        assert!(!command.arguments[1].single);
        assert!(command.arguments[1].optional);
        assert_eq!(
            command.arguments[1].default_source.as_deref(),
            Some("all players")
        );
    }

    #[test]
    fn command_arguments_are_available_to_body_expression_handlers() {
        let command = parse_command_header(
            "command /tell <target: player> <messages: strings>",
            &options(),
        )
        .unwrap();
        let context = InvocationContext {
            invocation_id: 1,
            subscription_id: "test".to_owned(),
            document_id: "file:///command.sk".to_owned(),
            document_revision: 1,
            expansion: None,
            syntax_context: 7,
        };

        let updates = command.context_updates(&context);
        let value = |key: &str| {
            updates
                .iter()
                .find(|update| update.key == key)
                .and_then(|update| update.value.as_deref())
                .map(|value| String::from_utf8_lossy(value).into_owned())
        };
        assert_eq!(value("core.command.argument-count").as_deref(), Some("2"));
        assert_eq!(
            value("core.command.argument.0.class").as_deref(),
            Some("org.bukkit.entity.Player")
        );
        assert_eq!(
            value("core.command.argument.1.single").as_deref(),
            Some("false")
        );
    }

    #[test]
    fn classifies_command_defaults_like_argument_new_instance() {
        let source = "  command test <target: player = %sender%> <message: string = \"hello %player%\"> <count: number = 1> <text: string = fallback>";
        let command = parse_command_header(source, &options_with_number()).unwrap();

        assert_eq!(command.defaults.len(), 3);

        let expression_default = &command.defaults[0];
        assert_eq!(expression_default.argument_index, 0);
        assert_eq!(expression_default.source, "%sender%");
        assert_eq!(expression_default.input, "sender");
        assert_eq!(expression_default.mode, DefaultParseMode::ExpressionsOnly);
        assert_eq!(expression_default.context, DefaultParseContext::Command);
        assert_eq!(
            expression_default.input_start,
            source.find("sender").expect("sender is present")
        );
        assert_eq!(
            expression_default.input_end,
            source.find("sender").unwrap() + "sender".len()
        );

        let string_default = &command.defaults[1];
        assert_eq!(string_default.argument_index, 1);
        assert_eq!(string_default.input, "\"hello %player%\"");
        assert_eq!(string_default.mode, DefaultParseMode::All);
        assert_eq!(string_default.context, DefaultParseContext::Default);
        assert_eq!(
            string_default.input_start,
            source.find("\"hello %player%\"").unwrap()
        );

        let literal_default = &command.defaults[2];
        assert_eq!(literal_default.argument_index, 2);
        assert_eq!(literal_default.input, "1");
        assert_eq!(literal_default.mode, DefaultParseMode::LiteralsOnly);
        assert_eq!(literal_default.context, DefaultParseContext::Default);

        // The unquoted String branch is SimpleLiteral in Skript and must not
        // create a host request, so `fallback` is intentionally absent above.
    }

    fn options_with_number() -> Vec<ExpressionTypeOption> {
        let mut options = options();
        options.push(option(
            "number",
            "number",
            "numbers",
            "java.lang.Number",
            &["DEFAULT", "COMMAND"],
        ));
        options
    }

    #[test]
    fn default_requests_override_event_context_and_keep_exact_input_spans() {
        let source = "command test <target: player = %sender%> <message: string = \"hello %player%\"> <count: number = 1>";
        let command = parse_command_header(source, &options_with_number()).unwrap();
        let header_start = 100;
        let parent = source_span(header_start, header_start + source.len() as u64);
        let requests = default_parse_requests(&parent, &parse_context(), &command, header_start);

        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| {
            request
                .expected_types
                .first()
                .is_some_and(|expected| expected.plural)
        }));
        assert_eq!(requests[0].request_id, 0);
        assert_eq!(requests[0].input, "sender");
        assert_eq!(
            requests[0].span.virtual_range.start,
            header_start + source.find("sender").unwrap() as u64
        );
        assert_eq!(
            requests[0].span.virtual_range.end,
            header_start + (source.find("sender").unwrap() + "sender".len()) as u64
        );
        assert_eq!(option_value(&requests[0], "parse.mode"), "expressions-only");
        assert_eq!(
            option_value(&requests[0], "context.value.parser.parse-context"),
            "COMMAND"
        );

        assert_eq!(requests[1].input, "\"hello %player%\"");
        assert_eq!(option_value(&requests[1], "parse.mode"), "all");
        assert_eq!(
            option_value(&requests[1], "context.value.parser.parse-context"),
            "DEFAULT"
        );
        assert_eq!(
            requests[1].span.virtual_range.start,
            header_start + source.find("\"hello %player%\"").unwrap() as u64
        );
        assert_eq!(
            requests[1].span.virtual_range.end,
            header_start
                + (source.find("\"hello %player%\"").unwrap() + "\"hello %player%\"".len()) as u64
        );

        assert_eq!(requests[2].input, "1");
        assert_eq!(option_value(&requests[2], "parse.mode"), "literals-only");
        assert_eq!(
            option_value(&requests[2], "context.value.parser.parse-context"),
            "DEFAULT"
        );
        assert_eq!(
            option_value(&requests[2], "context.event-classes"),
            COMMAND_EVENT
        );
        assert_eq!(
            option_value(&requests[2], "context.value.outer-value"),
            "kept"
        );
    }

    fn parse_request(request_id: u64, input: &str) -> ParseRequest {
        ParseRequest {
            request_id,
            parser_id: "host.expression".to_owned(),
            input: input.to_owned(),
            expected_types: Vec::new(),
            span: source_span(0, input.len() as u64),
            options: Vec::new(),
        }
    }

    fn parse_result(
        request: &ParseRequest,
        status: ParseResultStatus,
        roots: Vec<u64>,
    ) -> ParseResult {
        ParseResult {
            host_token: 0,
            request_id: request.request_id,
            parser_id: request.parser_id.clone(),
            status,
            roots,
            nodes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn accepts_only_complete_default_parse_results() {
        let request = parse_request(0, "1");
        assert!(
            validate_default_results(
                std::slice::from_ref(&request),
                &[parse_result(&request, ParseResultStatus::Success, vec![1])],
            )
            .is_ok()
        );
        assert!(
            validate_default_results(
                std::slice::from_ref(&request),
                &[parse_result(
                    &request,
                    ParseResultStatus::Failed,
                    Vec::new()
                )],
            )
            .is_err()
        );
        assert!(
            validate_default_results(
                std::slice::from_ref(&request),
                &[parse_result(
                    &request,
                    ParseResultStatus::Success,
                    Vec::new()
                )],
            )
            .is_err()
        );
    }

    #[test]
    fn a_default_starts_the_implicit_optional_suffix() {
        let command = parse_command_header(
            "command test <first: string = value> <second: string>",
            &options(),
        )
        .unwrap();
        assert!(command.arguments.iter().all(|argument| argument.optional));
    }

    #[test]
    fn rejects_invalid_brackets_names_and_parser_contexts() {
        assert!(parse_command_header("command test ]<string>", &options()).is_err());
        assert!(parse_command_header("command test <values::*: strings>", &options()).is_err());
        let default_only = [option(
            "string",
            "string",
            "strings",
            "java.lang.String",
            &["DEFAULT"],
        )];
        assert!(parse_command_header("command test <string>", &default_only).is_err());
    }

    #[test]
    fn empty_parse_contexts_remain_usable_but_explicitly_unresolved() {
        let unknown = [option(
            "string",
            "string",
            "strings",
            "java.lang.String",
            &[],
        )];
        let command = parse_command_header("command test <string>", &unknown).unwrap();
        assert!(!command.arguments[0].parse_context_known);
    }
}
