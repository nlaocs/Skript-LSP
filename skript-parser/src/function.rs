//! Skript-compatible parsing of registered Function calls.
//!
//! Call syntax and signature matching are independent from the source of a
//! [`FunctionDefinition`]. SSG catalog definitions are available today;
//! document-defined Functions can be supplied by an expression environment
//! later without introducing a second call parser.
//!
//! Name, argument, exact-signature, list-signature, and named-argument behavior
//! follows Skript 2.15.4's
//! [`FunctionReferenceParser`](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/common/function/FunctionReferenceParser.java)
//! and
//! [`FunctionArgumentParser`](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/common/function/FunctionArgumentParser.java).

use crate::expression::{
    ExpressionCandidate, ExpressionExpectedType, ExpressionNode, ExpressionNodeKind,
    ExpressionParseEnvironment, ExpressionParseError, ExpressionSession, PrefixParse,
};
use crate::pattern_match::{
    find_parenthesis_end, find_quote_end, find_variable_end, java_trim_range,
};
use crate::{FailureTrace, PatternFailure, PatternFailureReason, TextRange, choose_failure_trace};
use std::collections::{BTreeMap, HashSet};
use syntaxes::{ClassName, Function, Multiplicity, ParameterModifier};

/// One Function signature consumable by the native call parser.
///
/// Catalog-backed and future document-backed Functions use this same shape.
/// An environment-provided definition with the same parameter signature takes
/// precedence over the catalog definition, matching Skript's local namespace
/// lookup before its global namespace.
///
/// # Examples
///
/// An environment can expose a Function declared by the current document
/// without changing the call parser:
///
/// ```
/// use skript_parser::{FunctionDefinition, FunctionParameterDefinition};
/// use std::collections::BTreeMap;
/// use syntaxes::ClassName;
///
/// let definition = FunctionDefinition {
///     parser_id: "document.function".to_owned(),
///     name: "double".to_owned(),
///     definition_id: "document:function:double".to_owned(),
///     registration_id: "document:function:double:0".to_owned(),
///     registration_order: 0,
///     return_type: Some(ClassName("java.lang.Number".to_owned())),
///     return_type_is_single: true,
///     parameters: vec![FunctionParameterDefinition {
///         name: "value".to_owned(),
///         parameter_type: ClassName("java.lang.Number".to_owned()),
///         single: true,
///         optional: false,
///     }],
///     metadata: BTreeMap::new(),
/// };
/// assert_eq!(definition.name, "double");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub parser_id: String,
    pub name: String,
    pub definition_id: String,
    pub registration_id: String,
    pub registration_order: usize,
    pub return_type: Option<ClassName>,
    pub return_type_is_single: bool,
    pub parameters: Vec<FunctionParameterDefinition>,
    pub metadata: BTreeMap<String, String>,
}

impl FunctionDefinition {
    pub(crate) fn from_catalog(function: &Function) -> Self {
        Self {
            parser_id: "core.function".to_owned(),
            name: function.name.clone(),
            definition_id: function.definition_id.as_str().to_owned(),
            registration_id: function.registration_id.as_str().to_owned(),
            registration_order: function.registration_order,
            return_type: function.return_type.as_ref().map(component_type),
            return_type_is_single: function.return_type_is_single,
            parameters: function
                .parameters
                .iter()
                .map(|parameter| FunctionParameterDefinition {
                    name: parameter.name.clone(),
                    parameter_type: component_type(&parameter.parameter_type),
                    single: parameter.single,
                    optional: parameter.modifiers.contains(&ParameterModifier::Optional),
                })
                .collect(),
            metadata: BTreeMap::from([
                ("function.addon".to_owned(), function.addon.name.clone()),
                (
                    "function.addon-version".to_owned(),
                    function.addon.version.clone(),
                ),
            ]),
        }
    }

    pub(crate) fn shape(&self) -> Vec<(ClassName, bool)> {
        self.parameters
            .iter()
            .map(|parameter| (parameter.parameter_type.clone(), parameter.single))
            .collect()
    }

    fn return_multiplicity(&self) -> Multiplicity {
        if self.return_type_is_single {
            Multiplicity::Single
        } else {
            Multiplicity::Multiple
        }
    }

    fn is_list_signature(&self) -> bool {
        self.parameters.len() == 1 && !self.parameters[0].single
    }

    fn allows_named_arguments(&self) -> bool {
        self.metadata
            .get("function.named-arguments")
            .is_none_or(|value| value != "false")
    }
}

/// One parameter in a Function definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionParameterDefinition {
    pub name: String,
    pub parameter_type: ClassName,
    pub single: bool,
    pub optional: bool,
}

/// Request for non-catalog Function definitions visible in the current scope.
pub struct FunctionLookupRequest<'a> {
    pub name: &'a str,
    pub context: &'a crate::ExpressionParseContext,
}

/// Structured identity and argument mapping for one resolved Function call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: String,
    pub definition_id: String,
    pub registration_id: String,
    pub arguments: Vec<FunctionArgument>,
}

/// Mapping from a declared parameter to its parsed child Expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionArgument {
    pub parameter_name: String,
    pub supplied_name: Option<String>,
    pub child_start: usize,
    pub child_count: usize,
    pub omitted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgumentKind {
    Named,
    Unnamed,
}

#[derive(Debug, Clone)]
struct ParsedArgument {
    kind: ArgumentKind,
    name: Option<String>,
    value: TextRange,
    raw: TextRange,
}

#[derive(Debug)]
struct CallSyntax {
    name: String,
    arguments: Vec<ParsedArgument>,
    range: TextRange,
}

enum Selection {
    None,
    One(Box<ExpressionCandidate>),
    Ambiguous,
}

struct DefinitionParse {
    candidate: Option<ExpressionCandidate>,
    failure: Option<FailureTrace>,
}

struct ParameterParse {
    nodes: Option<Vec<ExpressionNode>>,
    failure: Option<FailureTrace>,
}

pub(crate) fn parse_function_call<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    candidate_ends: &[usize],
    expected_types: &[ExpressionExpectedType],
    depth: usize,
) -> Result<PrefixParse, ExpressionParseError> {
    let Some(call) = parse_call_syntax(
        session.source().virtual_source(),
        range,
        candidate_ends,
        session.function_policy(),
    ) else {
        return Ok(PrefixParse::default());
    };
    if duplicate_named_argument(&call.arguments) {
        return Ok(PrefixParse::default());
    }

    let definitions = session.function_definitions(&call.name)?;
    let definitions = definitions
        .iter()
        .filter(|definition| {
            definition.return_type.is_some()
                && session.return_type_matches(definition.return_type.as_ref(), expected_types)
                && session
                    .multiplicity_matches(Some(definition.return_multiplicity()), expected_types)
        })
        .collect::<Vec<_>>();
    if definitions.is_empty() {
        return Ok(PrefixParse::default());
    }

    let exact = definitions
        .iter()
        .copied()
        .filter(|definition| !definition.is_list_signature())
        .collect::<Vec<_>>();
    let (exact, exact_failure) = select_unique(session, &call, &exact, depth, false)?;
    match exact {
        Selection::One(candidate) => {
            return Ok(PrefixParse {
                candidates: vec![*candidate],
                failure: None,
            });
        }
        Selection::Ambiguous => return Ok(PrefixParse::default()),
        Selection::None => {}
    }

    let list = definitions
        .iter()
        .copied()
        .filter(|definition| definition.is_list_signature())
        .collect::<Vec<_>>();
    let (list, list_failure) = select_unique(session, &call, &list, depth, true)?;
    Ok(match list {
        Selection::One(candidate) => PrefixParse {
            candidates: vec![*candidate],
            failure: None,
        },
        Selection::None => PrefixParse {
            candidates: Vec::new(),
            failure: choose_failure_trace(exact_failure, list_failure),
        },
        Selection::Ambiguous => PrefixParse::default(),
    })
}

fn select_unique<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    call: &CallSyntax,
    definitions: &[&FunctionDefinition],
    depth: usize,
    list_signature: bool,
) -> Result<(Selection, Option<FailureTrace>), ExpressionParseError> {
    let mut selected = None;
    let mut selected_transaction_open = false;
    let mut failure = None;
    for definition in definitions {
        session
            .begin_semantic_candidate()
            .map_err(environment_error)?;
        let parsed = if list_signature {
            parse_list_definition(session, call, definition, depth)
        } else {
            parse_exact_definition(session, call, definition, depth)
        };
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                session
                    .finish_semantic_candidate(false)
                    .map_err(environment_error)?;
                if selected_transaction_open {
                    session
                        .finish_semantic_candidate(false)
                        .map_err(environment_error)?;
                }
                return Err(error);
            }
        };
        failure = choose_failure_trace(failure, parsed.failure);
        let Some(candidate) = parsed.candidate else {
            session
                .finish_semantic_candidate(false)
                .map_err(environment_error)?;
            continue;
        };
        if selected.is_none() {
            selected = Some(candidate);
            selected_transaction_open = true;
            continue;
        }

        session
            .finish_semantic_candidate(false)
            .map_err(environment_error)?;
        session
            .finish_semantic_candidate(false)
            .map_err(environment_error)?;
        return Ok((Selection::Ambiguous, failure));
    }

    if selected_transaction_open {
        session
            .finish_semantic_candidate(true)
            .map_err(environment_error)?;
    }
    Ok((
        selected.map_or(Selection::None, |candidate| {
            Selection::One(Box::new(candidate))
        }),
        failure,
    ))
}

fn parse_exact_definition<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    call: &CallSyntax,
    definition: &FunctionDefinition,
    depth: usize,
) -> Result<DefinitionParse, ExpressionParseError> {
    if call
        .arguments
        .iter()
        .any(|argument| argument.kind == ArgumentKind::Named)
        && !definition.allows_named_arguments()
    {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    }
    let required = definition
        .parameters
        .iter()
        .filter(|parameter| !parameter.optional)
        .count();
    if call.arguments.len() < required || call.arguments.len() > definition.parameters.len() {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    }
    let Some(mapped) = map_exact_arguments(&definition.parameters, &call.arguments) else {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    };
    build_candidate(session, call, definition, mapped, depth)
}

fn parse_list_definition<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    call: &CallSyntax,
    definition: &FunctionDefinition,
    depth: usize,
) -> Result<DefinitionParse, ExpressionParseError> {
    let parameter = &definition.parameters[0];
    if call
        .arguments
        .iter()
        .any(|argument| argument.kind == ArgumentKind::Named)
        && !definition.allows_named_arguments()
    {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    }
    if call.arguments.len() > 1
        && call
            .arguments
            .iter()
            .any(|argument| argument.kind == ArgumentKind::Named)
    {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    }
    if let [argument] = call.arguments.as_slice()
        && argument.kind == ArgumentKind::Named
        && argument.name.as_deref() != Some(parameter.name.as_str())
    {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    }
    if call.arguments.is_empty() && !parameter.optional {
        return Ok(DefinitionParse {
            candidate: None,
            failure: None,
        });
    }

    let supplied_name = call.arguments.first().and_then(|argument| {
        (argument.kind == ArgumentKind::Named)
            .then(|| argument.name.clone())
            .flatten()
    });
    // Skript parses all outer arguments of a one-parameter plural Function as
    // one Expression list. Keeping the commas here is what makes `(1,2),3`
    // a nested list (invalid) while allowing both `1,2` and `(1,2)`.
    let values = call
        .arguments
        .first()
        .zip(call.arguments.last())
        .map(|(first, last)| vec![TextRange::new(first.value.start, last.value.end)])
        .unwrap_or_default();
    build_candidate(
        session,
        call,
        definition,
        vec![MappedArgument {
            supplied_name,
            values,
        }],
        depth,
    )
}

#[derive(Debug)]
struct MappedArgument {
    supplied_name: Option<String>,
    values: Vec<TextRange>,
}

fn map_exact_arguments(
    parameters: &[FunctionParameterDefinition],
    arguments: &[ParsedArgument],
) -> Option<Vec<MappedArgument>> {
    let has_names = arguments
        .iter()
        .any(|argument| argument.kind == ArgumentKind::Named);
    let mut mapped = (0..parameters.len())
        .map(|_| MappedArgument {
            supplied_name: None,
            values: Vec::new(),
        })
        .collect::<Vec<_>>();
    if !has_names {
        for (index, argument) in arguments.iter().enumerate() {
            mapped[index].values.push(argument.value);
        }
        return required_parameters_are_present(parameters, &mapped).then_some(mapped);
    }

    let parameter_indexes = parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| (parameter.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    for argument in arguments {
        let Some(name) = argument.name.as_deref() else {
            continue;
        };
        if argument.kind == ArgumentKind::Named
            && let Some(index) = parameter_indexes.get(name).copied()
        {
            mapped[index].supplied_name = Some(name.to_owned());
            mapped[index].values.push(argument.value);
        }
    }

    let mut cursor = 0usize;
    for (argument_index, argument) in arguments.iter().enumerate() {
        if argument.kind == ArgumentKind::Named
            && let Some(index) = argument
                .name
                .as_deref()
                .and_then(|name| parameter_indexes.get(name))
                .copied()
        {
            cursor = cursor.max(index.saturating_add(1));
            continue;
        }

        let next_named = arguments[argument_index + 1..].iter().find_map(|next| {
            (next.kind == ArgumentKind::Named)
                .then(|| {
                    next.name
                        .as_deref()
                        .and_then(|name| parameter_indexes.get(name))
                        .copied()
                })
                .flatten()
        });
        let end = next_named.unwrap_or(parameters.len());
        let slot = (cursor..end).find(|index| mapped[*index].values.is_empty())?;
        mapped[slot]
            .values
            .push(if argument.kind == ArgumentKind::Named {
                argument.raw
            } else {
                argument.value
            });
        cursor = slot.saturating_add(1);
    }

    required_parameters_are_present(parameters, &mapped).then_some(mapped)
}

fn required_parameters_are_present(
    parameters: &[FunctionParameterDefinition],
    mapped: &[MappedArgument],
) -> bool {
    parameters
        .iter()
        .zip(mapped)
        .all(|(parameter, argument)| parameter.optional || !argument.values.is_empty())
}

fn build_candidate<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    call: &CallSyntax,
    definition: &FunctionDefinition,
    mapped: Vec<MappedArgument>,
    depth: usize,
) -> Result<DefinitionParse, ExpressionParseError> {
    let mut children = Vec::new();
    let mut arguments = Vec::with_capacity(definition.parameters.len());
    let mut failure = None;
    for (parameter, mapped) in definition.parameters.iter().zip(mapped) {
        let child_start = children.len();
        for value in mapped.values {
            let parsed = parse_parameter_value(session, value, parameter, depth + 1)?;
            failure = choose_failure_trace(failure, parsed.failure);
            let Some(mut nodes) = parsed.nodes else {
                return Ok(DefinitionParse {
                    candidate: None,
                    failure,
                });
            };
            children.append(&mut nodes);
        }
        let child_count = children.len() - child_start;
        arguments.push(FunctionArgument {
            parameter_name: parameter.name.clone(),
            supplied_name: mapped.supplied_name,
            child_start,
            child_count,
            omitted: child_count == 0,
        });
    }

    let mut metadata = definition.metadata.clone();
    metadata.insert("function.name".to_owned(), definition.name.clone());
    metadata.insert(
        "function.definition-id".to_owned(),
        definition.definition_id.clone(),
    );
    metadata.insert(
        "function.registration-id".to_owned(),
        definition.registration_id.clone(),
    );
    Ok(DefinitionParse {
        candidate: Some(ExpressionCandidate {
            node: ExpressionNode {
                kind: ExpressionNodeKind::Function {
                    parser_id: definition.parser_id.clone(),
                },
                function: Some(FunctionCall {
                    name: definition.name.clone(),
                    definition_id: definition.definition_id.clone(),
                    registration_id: definition.registration_id.clone(),
                    arguments,
                }),
                span: session.map_range(call.range)?,
                return_type: definition.return_type.clone(),
                possible_return_types: definition.return_type.iter().cloned().collect(),
                possible_return_types_state: if definition.return_type.is_some() {
                    syntaxes::PossibleReturnTypesState::Complete
                } else {
                    syntaxes::PossibleReturnTypesState::Unresolved
                },
                multiplicity: Some(definition.return_multiplicity()),
                captures: Vec::new(),
                tags: Vec::new(),
                mark: 0,
                children,
                routed_captures: Vec::new(),
                metadata,
            },
            expected_alternative: None,
        }),
        failure: None,
    })
}

fn parse_parameter_value<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    parameter: &FunctionParameterDefinition,
    depth: usize,
) -> Result<ParameterParse, ExpressionParseError> {
    if range.is_empty() {
        return Ok(ParameterParse {
            nodes: None,
            failure: None,
        });
    }
    let expected = [ExpressionExpectedType {
        class_name: parameter.parameter_type.clone(),
        plural: !parameter.single,
    }];
    let mut parsed =
        session.parse_prefixes_detailed(range, &[range.end], &expected, true, true, 0, depth)?;
    let Some(node) = (!parsed.candidates.is_empty()).then(|| parsed.candidates.remove(0).node)
    else {
        let failure = parsed.failure.or_else(|| {
            let expected = session
                .catalog()
                .types()
                .find(|value| value.original_class == parameter.parameter_type)
                .map_or_else(
                    || parameter.parameter_type.as_str().to_owned(),
                    |value| value.code_name.as_str().to_owned(),
                );
            session.map_range(range).ok().map(|span| {
                FailureTrace::leaf(PatternFailure {
                    span,
                    reasons: vec![PatternFailureReason::TypeExpression {
                        expected: vec![expected],
                    }],
                })
            })
        });
        return Ok(ParameterParse {
            nodes: None,
            failure,
        });
    };
    if parameter.single {
        return Ok(ParameterParse {
            nodes: Some(vec![node]),
            failure: None,
        });
    }
    Ok(ParameterParse {
        nodes: flatten_function_list(node),
        failure: None,
    })
}

fn flatten_function_list(mut node: ExpressionNode) -> Option<Vec<ExpressionNode>> {
    if !is_expression_list(&node) {
        return Some(vec![node]);
    }
    while matches!(node.kind, ExpressionNodeKind::Grouped) && node.children.len() == 1 {
        node = node.children.remove(0);
    }
    match node.kind {
        ExpressionNodeKind::List {
            conjunction: crate::ExpressionListConjunction::And,
        } if node.children.iter().any(is_expression_list) => None,
        ExpressionNodeKind::List {
            conjunction: crate::ExpressionListConjunction::And,
        } => Some(node.children),
        ExpressionNodeKind::List {
            conjunction: crate::ExpressionListConjunction::Or,
        } => None,
        _ => Some(vec![node]),
    }
}

fn is_expression_list(node: &ExpressionNode) -> bool {
    matches!(node.kind, ExpressionNodeKind::List { .. })
        || (matches!(node.kind, ExpressionNodeKind::Grouped)
            && node.children.len() == 1
            && is_expression_list(&node.children[0]))
}

fn parse_call_syntax(
    input: &str,
    range: TextRange,
    candidate_ends: &[usize],
    policy: crate::FunctionVersionPolicy,
) -> Option<CallSyntax> {
    let mut cursor = range.start;
    let first = input.get(cursor..range.end)?.chars().next()?;
    if first != '_' && !first.is_alphabetic() {
        return None;
    }
    cursor += first.len_utf8();
    while cursor < range.end {
        let ch = input.get(cursor..range.end)?.chars().next()?;
        if ch != '_' && !ch.is_alphabetic() && !ch.is_numeric() {
            break;
        }
        cursor += ch.len_utf8();
    }
    if input.get(cursor..range.end)?.chars().next()? != '(' {
        return None;
    }
    let close = find_parenthesis_end(input, cursor + '('.len_utf8(), range.end)?;
    let end = close + ')'.len_utf8();
    if !candidate_ends.contains(&end) {
        return None;
    }
    let name = input.get(range.start..cursor)?.to_owned();
    let argument_range = TextRange::new(cursor + '('.len_utf8(), close);
    Some(CallSyntax {
        name,
        arguments: split_arguments(input, argument_range, policy)?,
        range: TextRange::new(range.start, end),
    })
}

fn split_arguments(
    input: &str,
    range: TextRange,
    policy: crate::FunctionVersionPolicy,
) -> Option<Vec<ParsedArgument>> {
    if range.is_empty() {
        return Some(Vec::new());
    }
    let mut parts = Vec::new();
    let mut start = range.start;
    let mut cursor = range.start;
    let mut depth = 0usize;
    while cursor < range.end {
        let ch = input.get(cursor..range.end)?.chars().next()?;
        match ch {
            '"' => {
                cursor = find_quote_end(input, cursor + ch.len_utf8(), range.end)? + ch.len_utf8();
                continue;
            }
            '{' => {
                cursor =
                    find_variable_end(input, cursor + ch.len_utf8(), range.end)? + '}'.len_utf8();
                continue;
            }
            '(' => depth = depth.saturating_add(1),
            ')' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                if let Some(argument) =
                    parsed_argument(input, TextRange::new(start, cursor), policy)
                {
                    parts.push(argument);
                }
                start = cursor + ch.len_utf8();
            }
            _ => {}
        }
        cursor += ch.len_utf8();
    }
    if let Some(argument) = parsed_argument(input, TextRange::new(start, range.end), policy) {
        parts.push(argument);
    }
    Some(parts)
}

fn parsed_argument(
    input: &str,
    range: TextRange,
    policy: crate::FunctionVersionPolicy,
) -> Option<ParsedArgument> {
    let text = range.slice(input)?;
    let local = java_trim_range(text);
    if local.is_empty() {
        return None;
    }
    let trimmed = TextRange::new(range.start + local.start, range.start + local.end);
    let value = trimmed.slice(input)?;
    let named_end = if !policy.allow_named_arguments {
        None
    } else if policy.wide_named_argument_names {
        value.find(':').filter(|end| {
            *end > 0
                && !value[..*end]
                    .chars()
                    .any(|character| matches!(character, ':' | '(' | ')' | '{' | '}' | '"' | ','))
        })
    } else {
        let mut end = 0usize;
        for character in value.chars() {
            if character != '_' && !character.is_ascii_alphanumeric() {
                break;
            }
            end += character.len_utf8();
        }
        (end > 0 && value.get(end..).is_some_and(|tail| tail.starts_with(':'))).then_some(end)
    };
    if let Some(cursor) = named_end {
        let raw_value = TextRange::new(trimmed.start + cursor + 1, trimmed.end);
        let raw_text = raw_value.slice(input)?;
        let local_value = java_trim_range(raw_text);
        if !local_value.is_empty() {
            return Some(ParsedArgument {
                kind: ArgumentKind::Named,
                name: Some(value[..cursor].trim().to_owned()),
                value: TextRange::new(
                    raw_value.start + local_value.start,
                    raw_value.start + local_value.end,
                ),
                raw: trimmed,
            });
        }
    }
    Some(ParsedArgument {
        kind: ArgumentKind::Unnamed,
        name: None,
        value: trimmed,
        raw: trimmed,
    })
}

fn duplicate_named_argument(arguments: &[ParsedArgument]) -> bool {
    let mut names = HashSet::new();
    arguments.iter().any(|argument| {
        argument.kind == ArgumentKind::Named
            && argument
                .name
                .as_ref()
                .is_some_and(|name| !names.insert(name))
    })
}

fn component_type(class_name: &ClassName) -> ClassName {
    ClassName(
        class_name
            .as_str()
            .strip_suffix("[]")
            .unwrap_or(class_name.as_str())
            .to_owned(),
    )
}

fn environment_error(message: String) -> ExpressionParseError {
    ExpressionParseError::Environment { message }
}
