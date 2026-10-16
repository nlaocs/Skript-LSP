//! End-to-end parsing of one document revision.
//!
//! This module owns the transaction boundary that the individual host methods
//! intentionally leave to their callers. Syntax errors remain data in the
//! returned partial document. Extension faults retain their cause and uncertain
//! regions; fatal host failures, cancellation, and stale revisions roll back
//! both StateStore writes and document-scoped dynamic syntax.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use skript_parser::{
    ExpressionParseContext, FunctionRegistrySnapshot, MappedSource, RawTree, RawTreeOptions,
    StructureDocument, StructureParseRequest, StructureParserConfig, parse_raw_tree,
};
use syntaxes::{DynamicRegistryError, DynamicSyntaxSavepoint};

use crate::state::{CommitSummary, ParseTransaction, StateError};

use super::{
    AstMacroCall, AstMacroRequest, AstMacroResult, AstTree, ComponentFailure, HookCall,
    HookDecision, HookEffects, HostError, InvocationContext, MappedSpan, MatchSpan, ParserHost,
    ParserTextRange, TextMacroCall, TextMacroRequest, TextMacroResult, TreeMacroCall,
    TreeMacroRequest, TreeMacroResult, WasmStructureParseResult, ast_tree, empty_effects,
    extension_failure, mapped_span_to_wit, merge_effects,
};

/// Cooperative cancellation shared by the caller and one document parse.
///
/// Cancellation is observed between pipeline stages, inside native syntax
/// matching and before commit. A single running regex or WASM invocation still
/// returns under its own quotas before native probes can observe cancellation.
#[derive(Debug, Clone, Default)]
pub struct DocumentCancellationToken {
    cancelled: Arc<AtomicBool>,
    #[cfg(test)]
    native_probe_budget: Option<Arc<AtomicUsize>>,
}

impl DocumentCancellationToken {
    /// Creates a token in the active state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Calling this more than once is harmless.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(super) fn native_cancelled(&self) -> bool {
        #[cfg(test)]
        if let Some(budget) = &self.native_probe_budget
            && budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_sub(1)
                })
                .is_err()
        {
            self.cancel();
        }
        self.is_cancelled()
    }
}

/// Stage at which cooperative document cancellation was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentParseStage {
    Start,
    TextMacro,
    RawTree,
    TreeMacro,
    Syntax,
    AstMacro,
    Commit,
}

impl fmt::Display for DocumentParseStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Start => "start",
            Self::TextMacro => "text macro",
            Self::RawTree => "raw tree",
            Self::TreeMacro => "tree macro",
            Self::Syntax => "syntax",
            Self::AstMacro => "AST macro",
            Self::Commit => "commit",
        };
        formatter.write_str(name)
    }
}

/// Fatal failure of the end-to-end document pipeline.
#[derive(Debug, thiserror::Error)]
pub enum DocumentParseError {
    /// A host, parser, component, catalog, or transaction operation failed.
    #[error(transparent)]
    Host(HostError),
    /// This revision lost a race with a newer parse of the same document.
    #[error("document {document_id}@{actual} is stale; latest revision is {latest}")]
    StaleRevision {
        document_id: String,
        actual: u64,
        latest: u64,
    },
    /// The caller cancelled this document revision before it could commit.
    #[error("document {document_id}@{document_revision} was cancelled during {stage}")]
    Cancelled {
        document_id: String,
        document_revision: u64,
        stage: DocumentParseStage,
    },
}

impl From<HostError> for DocumentParseError {
    fn from(error: HostError) -> Self {
        match error {
            HostError::StateStore(StateError::StaleDocumentRevision {
                document_id,
                actual,
                latest,
            })
            | HostError::DynamicSyntax(DynamicRegistryError::StaleDocumentRevision {
                document_id,
                actual,
                latest,
            }) => Self::StaleRevision {
                document_id,
                actual,
                latest,
            },
            error => Self::Host(error),
        }
    }
}

/// Owned input for one complete document revision.
#[derive(Debug, Clone)]
pub struct DocumentParseRequest {
    pub project_uri: String,
    pub context: InvocationContext,
    pub source: MappedSource,
}

impl DocumentParseRequest {
    /// Creates an identity-mapped document request with the root syntax context.
    pub fn new(
        project_uri: impl Into<String>,
        document_id: impl Into<String>,
        document_revision: u64,
        source: impl Into<Arc<str>>,
    ) -> Self {
        let document_id = document_id.into();
        Self {
            project_uri: project_uri.into(),
            context: InvocationContext {
                invocation_id: document_revision,
                subscription_id: String::new(),
                document_id,
                document_revision,
                expansion: None,
                syntax_context: 0,
            },
            source: MappedSource::identity(source),
        }
    }

    /// Uses a caller-provided invocation context and mapped source.
    pub fn with_context(
        project_uri: impl Into<String>,
        context: InvocationContext,
        source: MappedSource,
    ) -> Self {
        Self {
            project_uri: project_uri.into(),
            context,
            source,
        }
    }
}

/// Version-sensitive lexer, nested syntax, and cancellation configuration.
#[derive(Debug, Clone)]
pub struct DocumentParserConfig {
    /// Explicit lexical behavior, or runtime-profile-derived behavior when absent.
    pub raw_tree: Option<RawTreeOptions>,
    /// Resource and nested parser configuration for the syntax phase.
    pub structure: StructureParserConfig,
    /// Cooperative cancellation checked at stage boundaries and native syntax probes.
    pub cancellation: DocumentCancellationToken,
}

impl Default for DocumentParserConfig {
    fn default() -> Self {
        Self {
            raw_tree: None,
            structure: StructureParserConfig::default(),
            cancellation: DocumentCancellationToken::new(),
        }
    }
}

/// Fully parsed partial AST and every accepted side effect for one committed revision.
/// Execution completeness is independent of ordinary input syntax errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentExecutionStatus {
    Complete,
    Partial,
}

/// Known data can be retained here without certifying the affected region's meaning.
#[derive(Debug, Clone)]
pub struct ExtensionFaultRegion {
    pub span: MappedSpan,
    pub fault: skript_parser::ExtensionFault,
}

#[derive(Debug)]
pub struct DocumentParseResult {
    pub execution_status: DocumentExecutionStatus,
    pub fault_regions: Vec<ExtensionFaultRegion>,
    pub source: MappedSource,
    pub raw_tree: RawTree,
    /// Native parser trace including unknown and recovered nodes.
    pub syntax: StructureDocument,
    /// Data-only AST; `fault_regions` identify semantics that remain uncertain.
    pub ast: AstTree,
    pub functions: FunctionRegistrySnapshot,
    pub effects: HookEffects,
    pub text_macro_decision: HookDecision,
    pub tree_macro_decision: HookDecision,
    pub ast_macro_decision: HookDecision,
    pub text_macro_calls: Vec<TextMacroCall>,
    pub tree_macro_calls: Vec<TreeMacroCall>,
    pub ast_macro_calls: Vec<AstMacroCall>,
    pub syntax_calls: Vec<HookCall>,
    pub component_failures: Vec<ComponentFailure>,
    pub state: CommitSummary,
}

struct UncommittedDocument {
    fault_regions: Vec<ExtensionFaultRegion>,
    source: MappedSource,
    raw_tree: RawTree,
    syntax: StructureDocument,
    ast: AstTree,
    functions: FunctionRegistrySnapshot,
    effects: HookEffects,
    text_macro_decision: HookDecision,
    tree_macro_decision: HookDecision,
    ast_macro_decision: HookDecision,
    text_macro_calls: Vec<TextMacroCall>,
    tree_macro_calls: Vec<TreeMacroCall>,
    ast_macro_calls: Vec<AstMacroCall>,
    syntax_calls: Vec<HookCall>,
    component_failures: Vec<ComponentFailure>,
}

impl ParserHost {
    /// Runs Text, RawTree, Tree, and two-pass Structure parsing atomically.
    ///
    /// Recoverable syntax failures remain in `syntax` as unknown or incomplete
    /// nodes. The method commits StateStore writes only after the entire current
    /// revision completes, and rolls back document-scoped dynamic registrations
    /// together with state when a fatal error or cancellation occurs.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use parser_wasm::{
    ///     DocumentParseError, DocumentParseRequest, DocumentParserConfig, ParserHost,
    /// };
    ///
    /// fn parse(mut host: ParserHost) -> Result<(), DocumentParseError> {
    ///     let result = host.parse_document(
    ///         DocumentParseRequest::new(
    ///             "file:///workspace",
    ///             "file:///workspace/main.sk",
    ///             1,
    ///             "on load:\n    send 1\n",
    ///         ),
    ///         DocumentParserConfig::default(),
    ///     )?;
    ///
    ///     assert_eq!(result.syntax.roots.len(), 1);
    ///     Ok(())
    /// }
    /// # let _ = parse;
    /// ```
    pub fn parse_document(
        &mut self,
        request: DocumentParseRequest,
        config: DocumentParserConfig,
    ) -> Result<DocumentParseResult, DocumentParseError> {
        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::Start,
        )?;
        let transaction = self.begin_parse(
            &request.project_uri,
            &request.context.document_id,
            request.context.document_revision,
        )?;
        let dynamic_savepoint = match self
            .dynamic_syntax_registry
            .as_ref()
            .map(|registry| {
                registry.savepoint(
                    &request.context.document_id,
                    request.context.document_revision,
                )
            })
            .transpose()
        {
            Ok(savepoint) => savepoint,
            Err(error) => {
                let _ = transaction.cancel();
                return Err(HostError::from(error).into());
            }
        };

        let parsed = self.parse_document_uncommitted(&transaction, &request, &config);
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                self.rollback_document(&transaction, dynamic_savepoint.as_ref())?;
                return Err(error);
            }
        };

        if let Err(error) = check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::Commit,
        ) {
            self.rollback_document(&transaction, dynamic_savepoint.as_ref())?;
            return Err(error);
        }

        let state = match transaction.commit() {
            Ok(state) => state,
            Err(error) => {
                self.rollback_document(&transaction, dynamic_savepoint.as_ref())?;
                return Err(HostError::from(error).into());
            }
        };

        Ok(DocumentParseResult {
            execution_status: if parsed.fault_regions.is_empty() {
                DocumentExecutionStatus::Complete
            } else {
                DocumentExecutionStatus::Partial
            },
            fault_regions: parsed.fault_regions,
            source: parsed.source,
            raw_tree: parsed.raw_tree,
            syntax: parsed.syntax,
            ast: parsed.ast,
            functions: parsed.functions,
            effects: parsed.effects,
            text_macro_decision: parsed.text_macro_decision,
            tree_macro_decision: parsed.tree_macro_decision,
            ast_macro_decision: parsed.ast_macro_decision,
            text_macro_calls: parsed.text_macro_calls,
            tree_macro_calls: parsed.tree_macro_calls,
            ast_macro_calls: parsed.ast_macro_calls,
            syntax_calls: parsed.syntax_calls,
            component_failures: parsed.component_failures,
            state,
        })
    }

    fn parse_document_uncommitted(
        &mut self,
        transaction: &ParseTransaction,
        request: &DocumentParseRequest,
        config: &DocumentParserConfig,
    ) -> Result<UncommittedDocument, DocumentParseError> {
        let document_state = transaction.savepoint().map_err(HostError::from)?;
        let document_dynamic = self
            .dynamic_syntax_registry
            .as_ref()
            .map(|registry| {
                registry.savepoint(
                    &request.context.document_id,
                    request.context.document_revision,
                )
            })
            .transpose()
            .map_err(HostError::from)?;
        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::TextMacro,
        )?;
        let TextMacroResult {
            decision: text_macro_decision,
            source,
            effects: text_effects,
            calls: text_macro_calls,
            failures: text_failures,
        } = self.expand_text_in_parse(
            transaction,
            TextMacroRequest {
                context: request.context.clone(),
                source: request.source.clone(),
            },
        )?;

        let global_macro_fault = !text_failures.is_empty();
        if global_macro_fault {
            transaction
                .rollback_to(&document_state)
                .map_err(HostError::from)?;
            if let (Some(registry), Some(savepoint)) =
                (&self.dynamic_syntax_registry, &document_dynamic)
            {
                registry.rollback_to(savepoint).map_err(HostError::from)?;
            }
        }
        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::RawTree,
        )?;
        let raw_options = config.raw_tree.unwrap_or_else(|| {
            raw_tree_options_for_runtime(self.config.runtime_profile.skript_version.as_deref())
        });
        let raw_tree = parse_raw_tree(&source, raw_options);

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::TreeMacro,
        )?;
        let TreeMacroResult {
            decision: tree_macro_decision,
            source,
            tree: raw_tree,
            effects: tree_effects,
            calls: tree_macro_calls,
            failures: tree_failures,
        } = if global_macro_fault {
            TreeMacroResult {
                decision: HookDecision::ContinueProcessing,
                source,
                tree: raw_tree,
                effects: empty_effects(),
                calls: Vec::new(),
                failures: Vec::new(),
            }
        } else {
            self.expand_tree_in_parse(
                transaction,
                TreeMacroRequest {
                    context: request.context.clone(),
                    source,
                    tree: raw_tree,
                },
            )?
        };

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::Syntax,
        )?;
        let mut fault_regions = document_fault_regions(&source, &text_failures, "Preprocess")?;
        fault_regions.extend(document_fault_regions(
            &source,
            &tree_failures,
            "TreeMacro",
        )?);
        let mut syntax_tree = raw_tree.clone();
        for node in &mut syntax_tree.nodes {
            let node_span = mapped_span_to_wit(node.span.clone());
            if global_macro_fault
                || fault_regions
                    .iter()
                    .any(|region| fault_contains(&region.span, &node_span))
            {
                node.kind = skript_parser::RawNodeKind::Invalid;
            }
        }
        let previous_cancellation = self
            .native_cancellation
            .replace(config.cancellation.clone());
        let parsed = self.parse_structures_in_parse(
            transaction,
            request.context.clone(),
            StructureParseRequest {
                source: &source,
                tree: &syntax_tree,
                context: ExpressionParseContext::default(),
            },
            config.structure.clone(),
        );
        self.native_cancellation = previous_cancellation;
        let WasmStructureParseResult {
            document: mut syntax,
            functions,
            effects: syntax_effects,
            calls: syntax_calls,
            failures: syntax_failures,
        } = parsed.map_err(|error| {
            if skript_parser::parse_cancellation(&error).is_some() {
                DocumentParseError::Cancelled {
                    document_id: request.context.document_id.clone(),
                    document_revision: request.context.document_revision,
                    stage: DocumentParseStage::Syntax,
                }
            } else {
                error.into()
            }
        })?;
        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::Syntax,
        )?;

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::AstMacro,
        )?;
        for region in &fault_regions {
            let range = ParserTextRange::new(
                region.span.virtual_range.start as usize,
                region.span.virtual_range.end as usize,
            );
            if range.is_valid_for(source.virtual_source()) {
                syntax.diagnostics.push(skript_parser::StructureDiagnostic {
                    raw_node_id: None,
                    kind: skript_parser::StructureDiagnosticKind::ExtensionFault,
                    span: MatchSpan {
                        local_range: range,
                        mapped: source.map_range(range).map_err(|error| {
                            HostError::SourceMapping {
                                message: error.to_string(),
                            }
                        })?,
                    },
                    message: region.fault.to_string(),
                });
            }
        }
        let ast = ast_tree::from_structure_document(&source, &syntax);
        let AstMacroResult {
            decision: ast_macro_decision,
            source,
            tree: ast,
            effects: ast_effects,
            calls: ast_macro_calls,
            failures: ast_failures,
        } = if global_macro_fault {
            AstMacroResult {
                decision: HookDecision::ContinueProcessing,
                source,
                tree: ast,
                effects: empty_effects(),
                calls: Vec::new(),
                failures: Vec::new(),
            }
        } else {
            self.expand_ast_in_parse(
                transaction,
                AstMacroRequest {
                    context: request.context.clone(),
                    source,
                    tree: ast,
                },
            )?
        };

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::AstMacro,
        )?;
        fault_regions.extend(document_fault_regions(&source, &syntax_failures, "Syntax")?);
        fault_regions.extend(document_fault_regions(&source, &ast_failures, "AstMacro")?);
        let mut effects = empty_effects();
        merge_effects(&mut effects, text_effects);
        merge_effects(&mut effects, tree_effects);
        merge_effects(&mut effects, syntax_effects);
        merge_effects(&mut effects, ast_effects);
        let mut component_failures = text_failures;
        component_failures.extend(tree_failures);
        component_failures.extend(syntax_failures);
        component_failures.extend(ast_failures);

        if global_macro_fault {
            transaction
                .rollback_to(&document_state)
                .map_err(HostError::from)?;
            effects.context_updates.clear();
            effects.parse_requests.clear();
            effects.parse_results.clear();
        }
        Ok(UncommittedDocument {
            fault_regions,
            source,
            raw_tree,
            syntax,
            ast,
            functions,
            effects,
            text_macro_decision,
            tree_macro_decision,
            ast_macro_decision,
            text_macro_calls,
            tree_macro_calls,
            ast_macro_calls,
            syntax_calls,
            component_failures,
        })
    }

    fn rollback_document(
        &self,
        transaction: &ParseTransaction,
        dynamic_savepoint: Option<&DynamicSyntaxSavepoint>,
    ) -> Result<(), HostError> {
        let dynamic_result = match (&self.dynamic_syntax_registry, dynamic_savepoint) {
            (Some(registry), Some(savepoint)) => registry.abort_document_to(savepoint),
            _ => Ok(()),
        };
        let state_result = transaction.cancel();
        dynamic_result?;
        state_result?;
        Ok(())
    }
}

fn check_cancelled(
    context: &InvocationContext,
    cancellation: &DocumentCancellationToken,
    stage: DocumentParseStage,
) -> Result<(), DocumentParseError> {
    if cancellation.is_cancelled() {
        return Err(DocumentParseError::Cancelled {
            document_id: context.document_id.clone(),
            document_revision: context.document_revision,
            stage,
        });
    }
    Ok(())
}

fn raw_tree_options_for_runtime(version: Option<&str>) -> RawTreeOptions {
    let Some(version) = version else {
        return RawTreeOptions::for_skript_version(2, 9);
    };
    let mut numbers = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse::<u32>().ok());
    let (Some(major), Some(minor)) = (numbers.next(), numbers.next()) else {
        return RawTreeOptions::for_skript_version(2, 9);
    };
    RawTreeOptions::for_skript_version(major, minor)
}

fn document_fault_regions(
    source: &MappedSource,
    failures: &[ComponentFailure],
    phase: &str,
) -> Result<Vec<ExtensionFaultRegion>, HostError> {
    let mut regions = Vec::new();
    for failure in failures {
        let span = match &failure.affected_span {
            Some(span) => span.clone(),
            None => mapped_span_to_wit(
                source
                    .map_range(ParserTextRange::new(0, source.virtual_source().len()))
                    .map_err(|error| HostError::SourceMapping {
                        message: error.to_string(),
                    })?,
            ),
        };
        regions.push(ExtensionFaultRegion {
            fault: extension_failure(failure, phase.to_owned(), "source region".to_owned()),
            span,
        });
    }
    Ok(regions)
}

fn fault_contains(fault: &MappedSpan, node: &MappedSpan) -> bool {
    fault.origins.iter().any(|affected| {
        node.origins.iter().any(|origin| {
            affected.original_range.start <= origin.original_range.start
                && origin.original_range.end <= affected.original_range.end
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostConfig;

    fn native_test_host() -> ParserHost {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/type-parser-versions/skript-2.16.2-java25");
        ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                syntax_catalog: Some(Arc::new(ssg::load(fixture).unwrap().into_catalog())),
                ..HostConfig::default()
            },
        )
        .unwrap()
    }

    fn native_token(budget: usize) -> DocumentCancellationToken {
        DocumentCancellationToken {
            native_probe_budget: Some(Arc::new(AtomicUsize::new(budget))),
            ..DocumentCancellationToken::new()
        }
    }

    #[test]
    fn native_cancellation_stops_direct_dispatch_before_the_first_guest() {
        use crate::host::{DispatchRequest, DispatchTarget, HookPayload, HookPhase};

        let mut host = native_test_host();
        let request =
            DocumentParseRequest::new("file:///workspace", "file:///native-dispatch.sk", 1, "");
        let transaction = host
            .begin_parse(&request.project_uri, &request.context.document_id, 1)
            .unwrap();
        let dispatch = || DispatchRequest {
            context: request.context.clone(),
            target: DispatchTarget::ParseStage,
            phase: HookPhase::Document,
            payload: HookPayload::Document(
                crate::bindings::nlaocs::skript_parser_addon::types::DocumentPayload {
                    text: String::new(),
                },
            ),
        };
        for component in &mut host.components {
            component.store.set_fuel(0).unwrap();
        }
        let token = DocumentCancellationToken::new();
        token.cancel();
        host.native_cancellation = Some(token);
        let error = host
            .dispatch_with_transaction(&transaction, dispatch())
            .unwrap_err();
        assert!(matches!(error, HostError::Cancelled(_)), "{error:?}");
        for component in &host.components {
            // prepare_store would replenish fuel before entering a guest.
            assert_eq!(component.store.get_fuel().unwrap(), 0);
            assert!(component.store.data().invocation.is_none());
            assert!(component.store.data().dynamic_syntax_update.is_none());
        }
        assert!(transaction.read_write_set().unwrap().writes.is_empty());
        transaction.savepoint().unwrap();

        // Prove the same target has a callable guest rather than an empty registry.
        host.native_cancellation = None;
        let control = host
            .dispatch_with_transaction(&transaction, dispatch())
            .unwrap();
        assert!(control.fault.is_none());
        assert!(control.failures.is_empty());
        assert_eq!(control.calls.len(), 1);
        assert_eq!(control.calls[0].component_id, "nlaocs.core-library");
        assert!(host.components[0].store.get_fuel().unwrap() > 0);
        transaction.cancel().unwrap();
    }

    #[test]
    fn builtin_additional_parse_shares_native_cancellation_with_its_new_session() {
        use crate::host::{ParseRequest, SubscriptionRegistry, WitParseResultStatus, WitTextRange};

        let mut host = native_test_host();
        // No guest dispatch can observe cancellation in this test. Only the
        // request entry and the new native ExpressionSession have probes.
        host.registry = SubscriptionRegistry::default();
        for component in &mut host.components {
            component.store.set_fuel(0).unwrap();
        }
        let document =
            DocumentParseRequest::new("file:///workspace", "file:///native-additional.sk", 1, "1");
        let transaction = host
            .begin_parse(&document.project_uri, &document.context.document_id, 1)
            .unwrap();
        let request = ParseRequest {
            request_id: 1,
            parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.to_owned(),
            input: "1".to_owned(),
            expected_types: Vec::new(),
            span: MappedSpan {
                virtual_range: WitTextRange { start: 0, end: 1 },
                origins: Vec::new(),
            },
            options: Vec::new(),
        };
        let token = native_token(1);
        host.native_cancellation = Some(token.clone());
        let next_token = host.next_parse_result_token;
        let error = host
            .execute_parse_requests(&transaction, &document.context, None, vec![request.clone()])
            .unwrap_err();
        assert!(
            matches!(
                error,
                HostError::ExpressionParser(skript_parser::ExpressionParseError::Cancelled(_))
            ),
            "{error:?}"
        );
        assert!(token.is_cancelled());
        assert!(host.active_parse_requests.is_empty());
        assert_eq!(host.next_parse_result_token, next_token);
        for component in &host.components {
            assert_eq!(component.store.get_fuel().unwrap(), 0);
            assert!(component.store.data().invocation.is_none());
        }
        assert!(transaction.read_write_set().unwrap().writes.is_empty());

        // The builtin request is otherwise valid and returns ordinary no-match
        // data without any registered leaf parser. Cancellation must be an Err.
        host.native_cancellation = None;
        let control = host
            .execute_parse_requests(&transaction, &document.context, None, vec![request])
            .unwrap();
        assert_eq!(control.len(), 1);
        assert!(matches!(
            control[0].wire.status,
            WitParseResultStatus::Failed
        ));
        assert_eq!(host.next_parse_result_token, next_token + 1);
        assert!(host.active_parse_requests.is_empty());
        transaction.cancel().unwrap();
    }

    #[test]
    fn native_cancellation_unwinds_nested_syntax_and_allows_the_next_revision() {
        let mut host = native_test_host();
        let sources = [
            "on join:\n    message \"hello\"\n",
            "function local(n: number) :: number:\n    return {_n}\non load:\n    send local(log(8)) to console\n",
            "command /canceltest:\n    trigger:\n        if 1 is 1:\n            send 1 + 2 * 3 to console\n",
        ];
        let mut revision = 0;
        for (index, source) in sources.into_iter().enumerate() {
            revision += 1;
            let token = native_token(usize::MAX);
            let baseline = host
                .parse_document(
                    DocumentParseRequest::new(
                        "file:///workspace",
                        "file:///native-cancel.sk",
                        revision,
                        source,
                    ),
                    DocumentParserConfig {
                        cancellation: token.clone(),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(baseline.component_failures.is_empty());
            assert!(baseline.syntax.diagnostics.is_empty());
            assert_eq!(baseline.functions.len(), usize::from(index == 1));
            let probes = usize::MAX
                - token
                    .native_probe_budget
                    .as_ref()
                    .unwrap()
                    .load(Ordering::Relaxed);
            assert!(probes > 2);
            for budget in [0, probes / 2, probes - 1] {
                revision += 1;
                let token = native_token(budget);
                let started = std::time::Instant::now();
                let error = host
                    .parse_document(
                        DocumentParseRequest::new(
                            "file:///workspace",
                            "file:///native-cancel.sk",
                            revision,
                            source,
                        ),
                        DocumentParserConfig {
                            cancellation: token.clone(),
                            ..Default::default()
                        },
                    )
                    .unwrap_err();
                println!(
                    "native-cancel,{index},{budget},{probes},{},cancelled",
                    started.elapsed().as_nanos()
                );
                assert!(
                    matches!(
                        error,
                        DocumentParseError::Cancelled {
                            stage: DocumentParseStage::Syntax,
                            ..
                        }
                    ),
                    "{error:?}"
                );
                assert!(token.is_cancelled());
                assert!(host.native_cancellation.is_none());
                assert!(host.active_parse_requests.is_empty());
                for component in &host.components {
                    assert!(component.store.data().invocation.is_none());
                    assert!(component.store.data().dynamic_syntax_update.is_none());
                }
                host.dynamic_syntax_registry
                    .as_ref()
                    .unwrap()
                    .begin_document_update("test.cleanup", 0, "file:///native-cancel.sk", revision)
                    .unwrap();
                revision += 1;
                let next = host
                    .parse_document(
                        DocumentParseRequest::new(
                            "file:///workspace",
                            "file:///native-cancel.sk",
                            revision,
                            "on load:\n    send 1 to console\n",
                        ),
                        DocumentParserConfig::default(),
                    )
                    .unwrap();
                assert!(next.component_failures.is_empty());
                assert!(next.syntax.diagnostics.is_empty());
                assert_eq!(next.functions.len(), 0);
                assert!(host.native_cancellation.is_none());
            }
        }
    }

    #[test]
    fn native_interruption_discards_pending_state_and_unfreezes_dynamic_syntax() {
        let mut host = native_test_host();
        host.state_store
            .register_component(
                "test.cleanup",
                &[crate::state::NamespaceDeclaration::private(
                    "native-cancel",
                    "test.cancel.v1",
                    1,
                )],
            )
            .unwrap();
        let request = DocumentParseRequest::new(
            "file:///workspace",
            "file:///native-state.sk",
            1,
            "on load:\n    send log(8) to console\n",
        );
        let transaction = host
            .begin_parse(&request.project_uri, &request.context.document_id, 1)
            .unwrap();
        let registry = host.dynamic_syntax_registry.as_ref().unwrap().clone();
        let savepoint = registry.savepoint(&request.context.document_id, 1).unwrap();
        let mut invocation = transaction.begin_invocation("test.cleanup").unwrap();
        invocation
            .put(
                crate::state::StateScope::Document,
                crate::state::NamespaceVisibility::Private,
                "native-cancel",
                "value",
                crate::state::StateValue::new(
                    "test.cancel.v1",
                    crate::state::StateEncoding::Raw,
                    b"pending".to_vec(),
                ),
            )
            .unwrap();
        invocation.commit().unwrap();
        assert!(!transaction.read_write_set().unwrap().writes.is_empty());
        let error = host
            .parse_document_uncommitted(
                &transaction,
                &request,
                &DocumentParserConfig {
                    cancellation: native_token(32),
                    ..Default::default()
                },
            )
            .err()
            .expect("native parsing must cancel");
        assert!(matches!(
            error,
            DocumentParseError::Cancelled {
                stage: DocumentParseStage::Syntax,
                ..
            }
        ));
        // The public document wrapper uses this same rollback after any native error.
        host.rollback_document(&transaction, Some(&savepoint))
            .unwrap();
        assert!(matches!(
            transaction.commit(),
            Err(StateError::TransactionClosed)
        ));
        registry
            .begin_document_update("test.cleanup", 0, &request.context.document_id, 1)
            .unwrap();
        let probe = host
            .begin_parse("file:///workspace", &request.context.document_id, 2)
            .unwrap();
        let mut invocation = probe.begin_invocation("test.cleanup").unwrap();
        assert!(
            invocation
                .get(
                    crate::state::StateScope::Document,
                    crate::state::NamespaceVisibility::Private,
                    "native-cancel",
                    "value"
                )
                .unwrap()
                .is_none()
        );
        invocation.rollback();
        probe.cancel().unwrap();
    }

    #[test]
    fn cancellation_after_syntax_freeze_closes_state_and_restores_registry() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let catalog = Arc::new(ssg::load(fixture).unwrap().into_catalog());
        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                syntax_catalog: Some(catalog),
                ..HostConfig::default()
            },
        )
        .unwrap();
        host.state_store
            .register_component(
                "test.cleanup",
                &[crate::state::NamespaceDeclaration::private(
                    "cancel-test",
                    "test.cancel.v1",
                    1,
                )],
            )
            .unwrap();
        let request = DocumentParseRequest::new(
            "file:///workspace",
            "file:///cancel.sk",
            1,
            "on load:\n    send 1 to console\n".to_owned(),
        );
        let config = DocumentParserConfig::default();
        let transaction = host
            .begin_parse(&request.project_uri, &request.context.document_id, 1)
            .unwrap();
        let registry = host.dynamic_syntax_registry.as_ref().unwrap().clone();
        let savepoint = registry.savepoint(&request.context.document_id, 1).unwrap();
        host.parse_document_uncommitted(&transaction, &request, &config)
            .unwrap();
        assert!(matches!(
            registry.rollback_to(&savepoint),
            Err(DynamicRegistryError::Frozen { .. })
        ));
        let mut invocation = transaction.begin_invocation("test.cleanup").unwrap();
        invocation
            .put(
                crate::state::StateScope::Document,
                crate::state::NamespaceVisibility::Private,
                "cancel-test",
                "value",
                crate::state::StateValue::new(
                    "test.cancel.v1",
                    crate::state::StateEncoding::Raw,
                    b"pending".to_vec(),
                ),
            )
            .unwrap();
        invocation.commit().unwrap();
        assert!(!transaction.read_write_set().unwrap().writes.is_empty());
        config.cancellation.cancel();
        assert!(matches!(
            check_cancelled(
                &request.context,
                &config.cancellation,
                DocumentParseStage::Commit
            ),
            Err(DocumentParseError::Cancelled {
                stage: DocumentParseStage::Commit,
                ..
            })
        ));
        host.rollback_document(&transaction, Some(&savepoint))
            .unwrap();
        assert!(matches!(
            transaction.commit(),
            Err(StateError::TransactionClosed)
        ));
        registry
            .begin_document_update("test.cleanup", 0, &request.context.document_id, 1)
            .unwrap();
        host.parse_document(
            DocumentParseRequest::new(
                "file:///workspace",
                "file:///cancel.sk",
                2,
                "on load:\n    send 1 to console\n".to_owned(),
            ),
            DocumentParserConfig::default(),
        )
        .unwrap();
        let probe = host
            .begin_parse("file:///workspace", "file:///cancel.sk", 3)
            .unwrap();
        let mut invocation = probe.begin_invocation("test.cleanup").unwrap();
        assert!(
            invocation
                .get(
                    crate::state::StateScope::Document,
                    crate::state::NamespaceVisibility::Private,
                    "cancel-test",
                    "value"
                )
                .unwrap()
                .is_none()
        );
        invocation.rollback();
        probe.cancel().unwrap();
    }
}
