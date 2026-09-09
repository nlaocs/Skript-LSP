//! End-to-end parsing of one document revision.
//!
//! This module owns the transaction boundary that the individual host methods
//! intentionally leave to their callers. Syntax errors remain data in the
//! returned partial document; host failures, cancellation, and stale revisions
//! roll back both StateStore writes and document-scoped dynamic syntax.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use skript_parser::{
    ExpressionParseContext, FunctionRegistrySnapshot, MappedSource, RawTree, RawTreeOptions,
    StructureDocument, StructureParseRequest, StructureParserConfig, parse_raw_tree,
};
use syntaxes::{DynamicRegistryError, DynamicSyntaxSavepoint};

use crate::state::{CommitSummary, ParseTransaction, StateError};

use super::{
    AstMacroCall, AstMacroRequest, AstMacroResult, AstTree, ComponentFailure, HookCall,
    HookDecision, HookEffects, HostError, InvocationContext, ParserHost, TextMacroCall,
    TextMacroRequest, TextMacroResult, TreeMacroCall, TreeMacroRequest, TreeMacroResult,
    WasmStructureParseResult, ast_tree, empty_effects, merge_effects,
};

/// Cooperative cancellation shared by the caller and one document parse.
///
/// Cancellation is observed between pipeline stages and immediately before the
/// atomic commit. A cancellation that arrives while native syntax matching is
/// running prevents commit as soon as that stage returns.
#[derive(Debug, Clone, Default)]
pub struct DocumentCancellationToken {
    cancelled: Arc<AtomicBool>,
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
    /// Cooperative cancellation checked at every document stage boundary.
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
#[derive(Debug)]
pub struct DocumentParseResult {
    pub source: MappedSource,
    pub raw_tree: RawTree,
    /// Native parser trace including unknown and recovered nodes.
    pub syntax: StructureDocument,
    /// Canonical data-only AST after every accepted AST macro.
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
        } = self.expand_tree_in_parse(
            transaction,
            TreeMacroRequest {
                context: request.context.clone(),
                source,
                tree: raw_tree,
            },
        )?;

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::Syntax,
        )?;
        let WasmStructureParseResult {
            document: syntax,
            functions,
            effects: syntax_effects,
            calls: syntax_calls,
            failures: syntax_failures,
        } = self.parse_structures_in_parse(
            transaction,
            request.context.clone(),
            StructureParseRequest {
                source: &source,
                tree: &raw_tree,
                context: ExpressionParseContext::default(),
            },
            config.structure.clone(),
        )?;

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::AstMacro,
        )?;
        let ast = ast_tree::from_structure_document(&source, &syntax);
        let AstMacroResult {
            decision: ast_macro_decision,
            source,
            tree: ast,
            effects: ast_effects,
            calls: ast_macro_calls,
            failures: ast_failures,
        } = self.expand_ast_in_parse(
            transaction,
            AstMacroRequest {
                context: request.context.clone(),
                source,
                tree: ast,
            },
        )?;

        check_cancelled(
            &request.context,
            &config.cancellation,
            DocumentParseStage::AstMacro,
        )?;
        let mut effects = empty_effects();
        merge_effects(&mut effects, text_effects);
        merge_effects(&mut effects, tree_effects);
        merge_effects(&mut effects, syntax_effects);
        merge_effects(&mut effects, ast_effects);
        let mut component_failures = text_failures;
        component_failures.extend(tree_failures);
        component_failures.extend(syntax_failures);
        component_failures.extend(ast_failures);

        Ok(UncommittedDocument {
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
            (Some(registry), Some(savepoint)) => registry.rollback_to(savepoint),
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
