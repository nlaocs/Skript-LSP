#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#![allow(missing_docs)]
//! Test-only grammar: wasm echo/log(value), sum/named(value; value).
//! IDs bind to the current nine-plugin CLI fixture; no Function is executed.
wit_bindgen::generate!({ path: "../../parser-wasm/wit", world: "parser-addon", generate_unused_types: true });
use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::{state_store, types::*};
use parser_wasm::{
    ABI_VERSION, CAPABILITY_ADDITIONAL_PARSE, CAPABILITY_EXPRESSION_PARSER, CAPABILITY_STATE_STORE,
};
const ID: &str = "test.function-addon";
struct FunctionAddon;
impl addon::Guest for FunctionAddon {
    fn manifest() -> ComponentManifest {
        ComponentManifest {
            component_id: ID.into(),
            component_version: "0.1.0".into(),
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: [
                CAPABILITY_EXPRESSION_PARSER,
                CAPABILITY_ADDITIONAL_PARSE,
                CAPABILITY_STATE_STORE,
            ]
            .into_iter()
            .map(|id| CapabilityRequirement {
                id: id.into(),
                minimum_version: 1,
                required: true,
            })
            .collect(),
            subscriptions: vec![HookSubscription {
                id: "function.leaf".into(),
                target: HookTarget::ParseStage,
                phase: HookPhase::Expression,
                priority: 10,
                mode: HookMode::Transform,
                capability_id: CAPABILITY_EXPRESSION_PARSER.into(),
                selector: HookSelector {
                    pattern_index: None,
                    pattern_source: None,
                    mark: None,
                    tags: vec![],
                    captures: vec![],
                    return_type: None,
                    multiplicity: None,
                    metadata: vec![],
                },
            }],
            registered_syntax_handlers: vec![],
            catalog_annotations: vec![],
            state_namespaces: vec![StateNamespaceDeclaration {
                name: "function-state".into(),
                visibility: StateNamespaceVisibility::Private,
                schema_id: "test.function-state".into(),
                schema_version: 1,
                readers: vec![],
                writers: vec![],
            }],
        }
    }
    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        parser_wasm::validate_compatibility(
            ABI_VERSION,
            parser_wasm::AbiVersion::new(profile.abi.major, profile.abi.minor),
            &[],
            &[],
        )
        .map_err(|error| CompatibilityError {
            kind: CompatibilityErrorKind::InvalidManifest,
            subject: ID.into(),
            message: error.to_string(),
        })
    }
}
impl hooks::Guest for FunctionAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let HookPayload::Expression(mut payload) = input.payload else {
            return Ok(noop());
        };
        if payload.active_type.is_some() || !payload.allow_expressions {
            return Ok(noop());
        }
        for end in payload.candidate_ends.iter().copied().rev() {
            let start = payload.remaining.start as usize;
            let Some(text) = payload.input.get(start..end as usize) else {
                continue;
            };
            let Some(body) = text.strip_prefix("wasm ") else {
                continue;
            };
            let Some((operation, body)) = body.split_once('(') else {
                continue;
            };
            let Some(body) = body.strip_suffix(')') else {
                continue;
            };
            let Some((name, definition, registration, return_type)) = identity(operation) else {
                continue;
            };
            let parts = if matches!(operation, "sum" | "named") {
                body.split(';').collect::<Vec<_>>()
            } else {
                vec![body]
            };
            let mut offset = start + text.len() - body.len() - 1;
            let requests = parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    let range = offset + part.len() - part.trim_start().len()
                        ..offset + part.trim_end().len();
                    offset += part.len() + 1;
                    ParseRequest {
                        request_id: index as u64,
                        parser_id: "host.expression".into(),
                        input: part.trim().into(),
                        expected_types: vec![ExpressionExpectedType {
                            class_name: if matches!(name, "dummy_echo" | "attachment") {
                                "java.lang.String"
                            } else {
                                "java.lang.Number"
                            }
                            .into(),
                            plural: false,
                        }],
                        span: subspan(&payload, range.start as u64, range.end as u64),
                        options: vec![],
                    }
                })
                .collect::<Vec<_>>();
            if input.parse_results.is_empty() {
                return Ok(HookOutput {
                    decision: HookDecision::ContinueProcessing,
                    replacement: Some(HookPayload::Expression(payload)),
                    effects: HookEffects {
                        parse_requests: requests,
                        ..effects()
                    },
                });
            }
            let mut children = Vec::new();
            for request in &requests {
                let Some(result) = input.parse_results.iter().find(|result| {
                    result.request_id == request.request_id
                        && result.parser_id == "host.expression"
                        && matches!(result.status, ParseResultStatus::Success)
                        && result.roots.len() == 1
                }) else {
                    return Ok(noop());
                };
                children.push(ParseResultReference {
                    host_token: result.host_token,
                    root_id: result.roots[0],
                });
            }
            let mut arguments = match operation {
                "sum" => vec![argument("ns", 0, children.len() as u64)],
                "log" => vec![argument("n", 0, 1), argument("base", 1, 0)],
                "named" if children.len() == 2 => {
                    children.reverse();
                    let mut n = argument("n", 0, 1);
                    n.supplied_name = Some("n".into());
                    let mut base = argument("base", 1, 1);
                    base.supplied_name = Some("base".into());
                    vec![n, base]
                }
                _ => vec![argument(
                    if name == "attachment" {
                        "object"
                    } else {
                        "text"
                    },
                    0,
                    1,
                )],
            };
            match operation {
                "bad-range" => {
                    arguments[0].child_start = u64::MAX;
                    arguments[0].child_count = 1;
                }
                "bad-overlap" => arguments.push(argument("other", 0, 1)),
                "bad-unowned" => arguments.clear(),
                "bad-omitted" => arguments[0].omitted = true,
                "bad-name" => arguments[0].parameter_name.clear(),
                _ => (),
            }
            let mut function = Some(FunctionCall {
                name: name.into(),
                definition_id: definition.into(),
                registration_id: registration.into(),
                arguments,
            });
            if operation.starts_with("quota-") {
                let call = function.as_mut().unwrap();
                let oversized = "名".repeat(1024 * 1024);
                match operation {
                    "quota-name" => call.name = oversized,
                    "quota-definition" => call.definition_id = oversized,
                    "quota-registration" => call.registration_id = oversized,
                    "quota-parameter" => call.arguments[0].parameter_name = oversized,
                    "quota-supplied" => call.arguments[0].supplied_name = Some(oversized),
                    _ => (),
                }
            }
            if operation == "bad-missing" {
                function = None;
            }
            if operation == "bad-reference" {
                children[0].host_token = u64::MAX;
            }
            let kind = if operation == "bad-kind" {
                ExpressionLeafKind::Custom
            } else {
                ExpressionLeafKind::Function
            };
            state_store::put(
                StateScope::Parse,
                StateNamespaceVisibility::Private,
                "function-state",
                if operation.starts_with("bad-") {
                    "invalid-function"
                } else {
                    "accepted-function"
                },
                &StateValue {
                    schema_id: "test.function-state".into(),
                    encoding: StateEncoding::Json,
                    bytes: b"true".to_vec(),
                },
            )
            .map_err(|error| error_message(error.message))?;
            payload.candidates.push(ExpressionLeafCandidate {
                parser_id: "test.function-addon.call".into(),
                kind,
                timing: ExpressionLeafTiming::BeforeRegistered,
                range: TextRange {
                    start: start as u64,
                    end: if operation == "bad-span" {
                        start as u64 + 1
                    } else {
                        end
                    },
                },
                return_type: Some(return_type.into()),
                multiplicity: Some(if name == "attachment" {
                    DynamicMultiplicity::Multiple
                } else {
                    DynamicMultiplicity::Single
                }),
                children,
                function,
                public_data: vec![],
                metadata: vec![],
            });
            return Ok(HookOutput {
                decision: HookDecision::ContinueProcessing,
                replacement: Some(HookPayload::Expression(payload)),
                effects: HookEffects {
                    parse_results: input.parse_results,
                    ..effects()
                },
            });
        }
        Ok(noop())
    }
}
fn identity(operation: &str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    Some(match operation {
        "echo" => echo(),
        "log" | "named" => (
            "log",
            "function:skript:69ad6a4e98d998d7604230e05157a36159f47ad4115861909708d27252d616a4",
            "function:skript:296e4b07677a3ad690d87becee2c2bb64dd19aa2f7dbb1ca1dd37a7d0ea3a5c9",
            "java.lang.Number",
        ),
        "sum" => (
            "sum",
            "function:skript:b3d3335bf53b23ed135b544f97e51fde9ff984f555f63e8e249f1b043bdc1eae",
            "function:skript:4beed31aa63f7c0f38a96b8fbeb8657545e8d512516bca063bc571283753e010",
            "java.lang.Number",
        ),
        "attachment" => (
            "attachment",
            "function:skjson:50930eba084cf9e5be3ac03349eddf45ccbb0131f6d6489ecfd45cafe31a7d24",
            "function:skjson:12654ec5ec5bdcac5b85d2dbf27ea4256ba891cbe7a9e6589bebccb43277cc54",
            "cz.coffeerequired.api.requests.Attachment",
        ),
        value if value.starts_with("bad-") || value.starts_with("quota-") => echo(),
        _ => return None,
    })
}
fn echo() -> (&'static str, &'static str, &'static str, &'static str) {
    (
        "dummy_echo",
        "function:skriptdummyaddon:948a5b697ef5decf8a620e3e7de76611899d172278a435cd0be0dfb0eb48d5fb",
        "function:skriptdummyaddon:2b50bbbc753e113dda4e71c130fb91e0f58a5bdc6b03a2c92a66aec9c50c359e",
        "java.lang.String",
    )
}
fn argument(name: &str, start: u64, count: u64) -> FunctionArgument {
    FunctionArgument {
        parameter_name: name.into(),
        supplied_name: None,
        child_start: start,
        child_count: count,
        omitted: count == 0,
    }
}
fn subspan(payload: &ExpressionPayload, start: u64, end: u64) -> MappedSpan {
    let left = start - payload.remaining.start;
    let right = end - payload.remaining.start;
    MappedSpan {
        virtual_range: TextRange {
            start: payload.span.virtual_range.start + left,
            end: payload.span.virtual_range.start + right,
        },
        origins: payload
            .span
            .origins
            .iter()
            .map(|origin| SourceOrigin {
                original_range: if origin.original_range.end - origin.original_range.start
                    >= payload.span.virtual_range.end - payload.span.virtual_range.start
                {
                    TextRange {
                        start: origin.original_range.start + left,
                        end: origin.original_range.start + right,
                    }
                } else {
                    origin.original_range
                },
                kind: origin.kind,
                expansion: origin.expansion,
            })
            .collect(),
    }
}
fn effects() -> HookEffects {
    HookEffects {
        diagnostics: vec![],
        context_updates: vec![],
        parse_requests: vec![],
        parse_results: vec![],
    }
}
fn noop() -> HookOutput {
    HookOutput {
        decision: HookDecision::NotApplicable,
        replacement: None,
        effects: effects(),
    }
}
fn error_message(message: String) -> AddonError {
    AddonError {
        kind: AddonErrorKind::InvalidPayload,
        message,
        diagnostics: vec![],
    }
}
impl text_macro::Guest for FunctionAddon {
    fn expand(_: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(error_message("unsupported".into()))
    }
}
impl tree_macro::Guest for FunctionAddon {
    fn expand(_: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(error_message("unsupported".into()))
    }
}
impl ast_macro::Guest for FunctionAddon {
    fn expand(_: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(error_message("unsupported".into()))
    }
}
#[cfg(target_arch = "wasm32")]
export!(FunctionAddon);
