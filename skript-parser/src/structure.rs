//! Top-level Structure parsing and EntryValidator enforcement.
use crate::{
    CandidateMatch, ExpressionExpectedType, ExpressionParseContext, ExpressionParseEnvironment,
    ExpressionParseError, ExpressionParserConfig, ExpressionSession, FailureFrame,
    FailureFrameRole, FailureTrace, HOST_CONDITION_PARSER_ID, HOST_EFFECT_PARSER_ID,
    HOST_EVENT_PARSER_ID, MappedSource, MatchSpan, ParsedCapture, ParsedCaptureResult,
    ParsedCaptureStatus, PatternCapture, PatternFailure, PatternFailureReason, RawNode, RawNodeId,
    RawNodeKind, RawTree, RegisteredSyntaxIdentity, SectionBodyNode, SectionDiagnostic, TextRange,
};
use std::collections::{BTreeMap, BTreeSet};
use syntaxes::{
    Catalog, ClassName, DynamicSyntaxSnapshot, EntryData, EntryKind, EntryValidator, NodeType,
    SyntaxKind,
};
use thiserror::Error;

mod entry_plans;
pub use entry_plans::*;

#[cfg(test)]
#[path="structure_body_reuse_tests.rs"]
mod body_reuse_tests;

/// Input for one complete top-level Structure pass.
pub struct StructureParseRequest<'a> {
    /// Mapped virtual source represented by `tree`.
    pub source: &'a MappedSource,
    /// Lossless indentation tree whose roots are Structure candidates.
    pub tree: &'a RawTree,
    /// Initial parser context inherited by each top-level Structure.
    pub context: ExpressionParseContext,
}

/// Resource limits used by nested pattern, Expression, Effect, and Section parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructureParserConfig {
    /// Shared recursive Expression parser configuration.
    pub expression: ExpressionParserConfig,
    /// Stop after accepted Structure header hooks, before parsing or exiting bodies.
    pub headers_only: bool,
}

/// Native body parser selected after a Structure header is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureBodyMode {
    /// The Structure is a Simple node and has no body.
    None,
    /// Preserve the body as lossless RawTree node IDs for WASM handling.
    Raw,
    /// Apply the registration's declarative EntryValidator.
    Entries,
    /// Recursively parse the body as Section and Effect statements.
    Trigger,
}

/// Monotonic editing-owner identity within one native Structure parse run.
///
/// A caller must also retain its document revision and parse-run identity.
pub type PreparationAttemptId = u64;

/// One current-source header attempt, before body validation and final adoption.
#[derive(Debug)]
pub struct StructureHeaderPreparation<'a> {
    /// Unique identity of this header attempt, including retries of the same pattern.
    pub attempt_id: PreparationAttemptId,
    /// Header accepted by the current ordered semantic hooks.
    pub candidate: &'a StructureCandidate,
    /// Body parser declared by those hooks, not a completed body result.
    pub body_mode: StructureBodyMode,
    /// More than one header interpretation remains; no unique layout is certified.
    pub ambiguous: bool,
}

/// Read-only editing proposals from the same active parse, never a committed AST.
#[derive(Debug)]
pub enum StructurePreparationEvent<'a> {
    /// Preprocessing and all root header hooks finished; bodies are still pending.
    Headers {
        /// Current transformed source with its original-source identity.
        source: &'a MappedSource,
        /// Current indentation tree after preprocessing.
        tree: &'a RawTree,
        /// Provisional owners, without types, variable history, or final adoption.
        headers: Vec<StructureHeaderPreparation<'a>>,
    },
    /// A later Structure enter hook accepted a new attempt after an earlier rejection.
    StructureHeader {
        /// Current mapped source and original-source identity.
        source: &'a MappedSource,
        /// The same current indentation tree used by the active parser.
        tree: &'a RawTree,
        /// Provisional owner after its enter hook, before body validation.
        header: StructureHeaderPreparation<'a>,
    },
    /// A Section enter hook accepted this exact candidate and its body semantics.
    SectionHeader {
        /// Current mapped source and original-source identity.
        source: &'a MappedSource,
        /// The same current indentation tree used by the active parser.
        tree: &'a RawTree,
        /// Header, semantic body mode, and context before any child is parsed.
        candidate: &'a crate::SectionCandidate,
        /// Fresh attempt, even when node, registration, and pattern are unchanged.
        attempt_id: PreparationAttemptId,
        /// Exact active parent attempt; an uncertified parent cannot be bypassed.
        parent_attempt_id: Option<PreparationAttemptId>,
    },
    /// A prepared Structure or Section attempt failed; revoke its descendants too.
    Invalidated {
        /// Exact attempt being revoked, not merely its source location.
        attempt_id: PreparationAttemptId,
        /// Root containing the affected layout.
        raw_node_id: RawNodeId,
        /// Exact registration of the rejected attempt.
        registration_id: &'a str,
        /// Pattern within that registration.
        pattern_index: usize,
    },
}

/// Lifecycle point at which a Structure hook is invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureHookTiming {
    /// After the header matched and before its body is parsed.
    EnterBody,
    /// After the chosen native body parser completed.
    ExitBody,
}

/// Parser-neutral Structure lifecycle request exposed to native and WASM environments.
pub struct StructureHookRequest<'a> {
    /// Complete virtual document source.
    pub input: &'a str,
    /// Lossless document tree; `candidate.raw_node_id` identifies the Structure root.
    pub tree: &'a RawTree,
    /// Current lifecycle point.
    pub timing: StructureHookTiming,
    /// Candidate and any body result available at this lifecycle point.
    pub candidate: &'a StructureCandidate,
    /// Context visible to this Structure and its body.
    pub context: &'a ExpressionParseContext,
    /// Body mode inferred from NodeType and static or dynamic EntryValidator metadata.
    pub default_body_mode: StructureBodyMode,
}

/// Addon decision made before parsing a selected Structure body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureHookDecision {
    /// Keep the candidate and parse its body with the returned context and mode.
    Accept {
        /// Context scoped to the Structure body.
        context: ExpressionParseContext,
        /// Native body parser to run.
        body_mode: StructureBodyMode,
        /// Section entry paths whose bodies the semantic owner parses as executable code.
        entry_body_modes: BTreeMap<Vec<String>, crate::SectionBodyMode>,
        /// Candidate metadata after ordered addon hooks ran.
        metadata: BTreeMap<String, String>,
    },
    /// Discard this candidate and continue recovery.
    Reject {
        /// Human-readable rejection reason supplied by the environment.
        reason: String,
        /// Candidate-owned details promoted only when this rejection wins.
        diagnostics: Vec<crate::SemanticDiagnostic>,
    },
}

/// One static or dynamic Structure registration accepted by its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureCandidate {
    /// RawTree root claimed by this candidate.
    pub raw_node_id: RawNodeId,
    /// Full registered-pattern result and stable SSG identities.
    pub matched: CandidateMatch,
    /// Java implementation class for a static registration.
    pub element_class: Option<ClassName>,
    /// Node shape declared by SSG, or `Both` for dynamic registrations.
    pub declared_node_type: NodeType,
    /// Actual Simple or Section shape found in the RawTree.
    pub actual_node_type: NodeType,
    /// Regex and typed captures delegated to registered parser routes.
    pub parsed_captures: Vec<ParsedCapture>,
    /// Body representation selected by native defaults or a WASM hook.
    pub body: StructureBody,
    /// Native entry validation failed; retained children are editor recovery only.
    pub entry_validation_failed: bool,
    /// Frozen declaration used by this exact static or dynamic candidate.
    pub entry_validator: Option<EntryValidator>,
    /// Validated Section entry paths and their semantic body parsers.
    pub entry_body_modes: BTreeMap<Vec<String>, crate::SectionBodyMode>,
    /// Opaque handler selected by a dynamic registration.
    pub handler: Option<String>,
    /// Dynamic and addon-owned candidate metadata.
    pub metadata: BTreeMap<String, String>,
}

/// Parsed or losslessly retained contents of one Structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureBody {
    /// A Simple Structure or an explicitly bodyless Section.
    None,
    /// Uninterpreted direct child RawTree node IDs.
    Raw(Vec<RawNodeId>),
    /// Declarative EntryValidator output in source/default order.
    Entries(Vec<StructureEntry>),
    /// Recursively parsed statements used by Event and Function structures.
    Trigger(Vec<SectionBodyNode>),
}

/// One matched or defaulted EntryValidator value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureEntry {
    /// Source RawTree node, or `None` for a defaulted value.
    pub raw_node_id: Option<RawNodeId>,
    /// Index in the owning (possibly nested) EntryValidator's ordered declarations.
    pub declaration_index: usize,
    /// Exact key spelling mapped through SourceMap, absent for implicit defaults.
    pub key_span: Option<MatchSpan>,
    /// EntryData key declared by the registration.
    pub key: String,
    /// Java EntryData implementation class retained for addon dispatch.
    pub entry_data_class: ClassName,
    /// Normalized EntryData family recovered by SSG.
    pub kind: EntryKind,
    /// Complete source line, or a serialized default value.
    pub source: String,
    /// Mapped source span; defaults use a zero-width body-end span.
    pub span: MatchSpan,
    /// Whether this value came from EntryData.defaultValue.
    pub defaulted: bool,
    /// Parsed semantic value or lossless fallback.
    pub value: StructureEntryValue,
}

/// Semantic value produced for a Structure entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureEntryValue {
    /// Key/value text requiring no native semantic parser.
    Raw(String),
    /// Literal or Expression parsed through the shared ExpressionSession.
    Expression(Box<crate::ExpressionNode>),
    /// Trigger body parsed as nested Sections and Effects.
    Trigger(Vec<SectionBodyNode>),
    /// Nested EntryValidator result.
    Container(Vec<StructureEntry>),
    /// Section body retained as RawTree node IDs.
    Section(Vec<RawNodeId>),
    /// Addon-defined EntryData retained for WASM interpretation.
    Unknown(String),
}

/// Recoverable problem found while claiming a Structure or validating its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureDiagnosticKind {
    /// An extension failed; syntax adoption is not determined for this node.
    ExtensionFault,
    /// No registered Structure or EntryData accepted the node.
    Unclaimed,
    /// More than one registration accepted the same header.
    MultipleClaims,
    /// A non-optional EntryData value was absent.
    MissingRequiredEntry,
    /// A non-multiple EntryData value appeared again.
    DuplicateEntry,
    /// A known EntryData parser could not parse its value.
    InvalidEntryValue,
    /// SSG retained an addon-specific EntryData that requires WASM semantics.
    UnknownEntryData,
}

/// Source-mapped diagnostic that does not discard the partial Structure tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureDiagnostic {
    /// Related RawTree node when one exists.
    pub raw_node_id: Option<RawNodeId>,
    /// Stable diagnostic category.
    pub kind: StructureDiagnosticKind,
    /// Primary mapped diagnostic location.
    pub span: MatchSpan,
    /// Human-readable explanation.
    pub message: String,
}

/// Top-level source node accepted by no Structure registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownStructureNode {
    /// Unclaimed RawTree root.
    pub raw_node_id: RawNodeId,
    /// Exact header source excluding a Section's trailing colon.
    pub source: String,
    /// Mapped span of `source`.
    pub span: MatchSpan,
    /// Farthest registered-pattern failure retained for diagnostics.
    pub failure: Option<FailureTrace>,
    /// Best header/body candidate retained for editor recovery, but not accepted.
    pub partial: Option<Box<StructureCandidate>>,
}

/// Selected Structure, complete alternatives, and recovery information for one root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureMatches {
    /// Highest-ranked candidate whose header and enter hook were accepted.
    pub selected: Option<StructureCandidate>,
    /// Other registrations that matched the same header.
    pub alternatives: Vec<StructureCandidate>,
    /// Source-preserving result when no candidate was accepted.
    pub unknown: Option<UnknownStructureNode>,
    /// Header-level ambiguity and recovery diagnostics.
    pub diagnostics: Vec<StructureDiagnostic>,
}

/// Result for one top-level RawTree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureDocumentNode {
    /// A claimed or recoverably unclaimed Structure header.
    Structure(Box<StructureMatches>),
    /// A blank or comment root retained by node ID.
    Trivia(RawNodeId),
    /// An invalid RawTree root retained without Structure matching.
    Unclaimed(RawNodeId),
}

/// Complete first-pass/second-pass Structure result for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureDocument {
    /// Top-level nodes in source order.
    pub roots: Vec<StructureDocumentNode>,
    /// Header, body, and EntryValidator diagnostics in source order.
    pub diagnostics: Vec<StructureDiagnostic>,
}

/// Fatal failure that prevents a trustworthy partial Structure document.
#[derive(Debug, Clone, Error)]
pub enum StructureParseError {
    /// A requested range is outside the source or splits UTF-8.
    #[error("Structure range {range} is invalid for the mapped source")]
    InvalidRange { range: TextRange },
    /// A non-trivia node lacks the code span needed for matching.
    #[error("Structure RawTree node {node_id} has no code span")]
    MissingCodeSpan { node_id: RawNodeId },
    /// A RawTree Section did not retain its expected trailing colon.
    #[error("Structure Section header does not end with a colon")]
    MissingHeaderColon,
    /// Recursive Expression matching failed.
    #[error(transparent)]
    Expression(#[from] ExpressionParseError),
    /// A nested Effect, Section, or Entry body parser failed.
    #[error("Structure body parser failed: {message}")]
    Body { message: String },
    /// Native or WASM environment processing failed.
    #[error("Structure parser extension failed: {message}")]
    Environment { message: String },
}

struct PendingStructure {
    root_index: usize,
    node_id: RawNodeId,
    parent_context: ExpressionParseContext,
    context: ExpressionParseContext,
    body_mode: StructureBodyMode,
    preparation_attempt: PreparationAttemptId,
}

struct ParsedStructureBody {
    body: StructureBody,
    diagnostics: Vec<StructureDiagnostic>,
    entries_valid: bool,
}

type StructureBodyRejection = Option<(String, Vec<crate::SemanticDiagnostic>)>;
type StructureBodyAttempt = Result<
    (
        StructureCandidate,
        Vec<StructureDiagnostic>,
        StructureBodyRejection,
    ),
    StructureParseError,
>;

/// Parses all top-level roots using static SSG Structure registrations.
pub fn parse_structures<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    request: StructureParseRequest<'_>,
    environment: &mut E,
    config: StructureParserConfig,
) -> Result<StructureDocument, StructureParseError> {
    parse_structures_with_snapshot(catalog, None, request, environment, config)
}

/// Parses all top-level roots using static and frozen dynamic registrations.
///
/// Header hooks run for every selected root before any body is parsed, matching
/// Skript's two-pass Structure initialization order. Body hooks then run with
/// Structure-scoped context while preserving partial results and diagnostics.
pub fn parse_structures_with_snapshot<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    request: StructureParseRequest<'_>,
    environment: &mut E,
    config: StructureParserConfig,
) -> Result<StructureDocument, StructureParseError> {
    let StructureParserConfig {
        expression,
        headers_only,
    } = config;
    let measurement = environment.measurement_scope("native.session_prepare");
    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        expression,
    );
    drop(measurement);
    let parsed =
        parse_structures_with_session(&mut session, request.source, request.tree, headers_only);
    if parsed.is_err() {
        session.invalidate_editing_preparations();
    }
    parsed
}

fn parse_structures_with_session<E: ExpressionParseEnvironment>(
    mut session: &mut ExpressionSession<'_, E>,
    source: &MappedSource,
    tree: &RawTree,
    headers_only: bool,
) -> Result<StructureDocument, StructureParseError> {
    let mut roots = Vec::with_capacity(tree.roots.len());
    let mut diagnostics = Vec::new();
    let mut pending = Vec::new();

    // Skript initializes every top-level Structure before loading any body.
    for node_id in &tree.roots {
        session.check_cancelled()?;
        let Some(node) = tree.get(*node_id) else {
            continue;
        };
        match node.kind {
            RawNodeKind::Blank | RawNodeKind::Comment => {
                roots.push(StructureDocumentNode::Trivia(node.id));
            }
            RawNodeKind::Simple | RawNodeKind::Section => {
                let parent_context = session.context().clone();
                let checkpoint = session
                    .begin_recovery_scope()
                    .map_err(|message| session.environment_error(message))?;
                let measurement = session.environment().measurement_scope("native.structure_header");
                let header = parse_structure_header(&mut session, tree, node);
                drop(measurement);
                session
                    .finish_recovery_scope(checkpoint, header.is_ok())
                    .map_err(|message| session.environment_error(message))?;
                let (matches, selected_context, body_mode) = match header {
                    Ok(value) => value,
                    Err(error) if crate::function_namespace_unavailable(&error).is_some() => {
                        let unavailable = crate::function_namespace_unavailable(&error)
                            .expect("guard checked dependency");
                        let span = session.map_range(node.span.virtual_range)?;
                        session
                            .environment_mut()
                            .observe_function_namespace_unavailable(unavailable, span.mapped);
                        session.replace_context(parent_context);
                        roots.push(StructureDocumentNode::Unclaimed(node.id));
                        continue;
                    }
                    Err(error) if crate::extension_fault(&error).is_some() => {
                        let fault = crate::extension_fault(&error).expect("guard checked fault");
                        let span = session.map_range(
                            node.code_span
                                .as_ref()
                                .map_or(node.span.virtual_range, |span| span.virtual_range),
                        )?;
                        session
                            .environment_mut()
                            .observe_extension_fault(fault, span.mapped);
                        session.replace_context(parent_context);
                        diagnostics.push(node_diagnostic(
                            &session,
                            node,
                            StructureDiagnosticKind::ExtensionFault,
                            &error.to_string(),
                        )?);
                        roots.push(StructureDocumentNode::Unclaimed(node.id));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                diagnostics.extend(matches.diagnostics.iter().cloned());
                let root_index = roots.len();
                roots.push(StructureDocumentNode::Structure(Box::new(matches)));
                if let (Some(context), Some(body_mode)) = (selected_context, body_mode) {
                    pending.push(PendingStructure {
                        root_index,
                        node_id: node.id,
                        parent_context,
                        context,
                        body_mode,
                        preparation_attempt: session.next_editing_attempt()?,
                    });
                }
            }
            RawNodeKind::Invalid => {
                diagnostics.push(node_diagnostic(
                    &session,
                    node,
                    StructureDiagnosticKind::Unclaimed,
                    "invalid top-level node is not a Structure",
                )?);
                roots.push(StructureDocumentNode::Unclaimed(node.id));
            }
        }
    }

    let prepared_headers = pending
        .iter()
        .filter_map(|item| {
            let StructureDocumentNode::Structure(matches) = &roots[item.root_index] else {
                return None;
            };
            Some(StructureHeaderPreparation {
                attempt_id: item.preparation_attempt,
                candidate: matches.selected.as_ref()?,
                body_mode: item.body_mode,
                ambiguous: !matches.alternatives.is_empty(),
            })
        })
        .collect();
    session.publish_preparation(StructurePreparationEvent::Headers {
        source,
        tree,
        headers: prepared_headers,
    });

    if headers_only {
        return Ok(StructureDocument { roots, diagnostics });
    }

    for item in pending {
        session.check_cancelled()?;
        let Some(StructureDocumentNode::Structure(matches)) = roots.get_mut(item.root_index) else {
            continue;
        };
        let Some(node) = tree.get(item.node_id) else {
            continue;
        };
        let Some(selected) = matches.selected.take() else {
            continue;
        };
        let document_context = session.context().clone();
        let initially_unambiguous=matches.alternatives.is_empty();
        let mut candidates = Vec::with_capacity(matches.alternatives.len() + 1);
        candidates.push((selected, Some((item.context, item.body_mode))));
        candidates.extend(
            matches
                .alternatives
                .drain(..)
                .map(|candidate| (candidate, None)),
        );
        let mut best_rejection = None;

        while !candidates.is_empty() {
            session.check_cancelled()?;
            let (mut candidate, initialized) = candidates.remove(0);
            let retry = initialized.is_none();
            let preparation_attempt = if retry {
                session.next_editing_attempt()?
            } else {
                item.preparation_attempt
            };
            session.replace_context(document_context.clone());
            session
                .begin_semantic_candidate()
                .map_err(|message| session.environment_error(message))?;
            let provisional_candidate = candidate.clone();
            let mut body_boundary_entered=false;
            let attempt: StructureBodyAttempt = (|| {
                let (context, body_mode) = if let Some(initialized) = initialized {
                    initialized
                } else {
                    session.replace_context(item.parent_context.clone());
                    let default_body_mode = default_body_mode(&session, &candidate);
                    let hook_context = session.context().clone();
                    let enter = StructureHookRequest {
                        input: session.source().virtual_source(),
                        tree,
                        timing: StructureHookTiming::EnterBody,
                        candidate: &candidate,
                        context: &hook_context,
                        default_body_mode,
                    };
                    match session
                        .environment_mut()
                        .enter_structure(enter)
                        .map_err(|message| session.environment_error(message))?
                    {
                        StructureHookDecision::Accept {
                            context,
                            body_mode,
                            entry_body_modes,
                            metadata,
                        } => {
                            candidate.entry_body_modes = entry_body_modes;
                            candidate.metadata = metadata;
                            (context, body_mode)
                        }
                        StructureHookDecision::Reject {
                            reason,
                            diagnostics,
                        } => {
                            return Ok((candidate, Vec::new(), Some((reason, diagnostics))));
                        }
                    }
                };
                if retry {
                    session.publish_preparation(StructurePreparationEvent::StructureHeader {
                        source,
                        tree,
                        header: StructureHeaderPreparation {
                            attempt_id: preparation_attempt,
                            candidate: &candidate,
                            body_mode,
                            ambiguous: !candidates.is_empty(),
                        },
                    });
                }
                let validator = candidate.entry_validator.clone();
                let saved_context = session.replace_context(context);
                // Diagnostic first-failure only. Keep each original predicate's
                // order and short circuit; never re-run an environment query.
                drop(crate::PatternMatchEnvironment::measurement_scope(session,"body_admission.native.total"));
                let mut admission="body_admission.native.reject.mode";
                let observed=body_mode==StructureBodyMode::Trigger
                    && {admission="body_admission.native.reject.initial_ambiguity"; initially_unambiguous}
                    && {admission="body_admission.native.reject.retry"; !retry}
                    && {admission="body_admission.native.reject.remaining_candidates"; candidates.is_empty()}
                    && {admission="body_admission.native.reject.header_captures"; candidate.parsed_captures.is_empty()}
                    && {admission="body_admission.native.reject.recovery"; !session.body_config().matcher.recover_type_expression_failures}
                    && {admission="body_admission.native.reject.environment"; session.environment_mut().structure_body_reuse_enabled()};
                drop(crate::PatternMatchEnvironment::measurement_scope(session,
                    if observed {"body_admission.native.passed"} else {admission}));
                let parsed=session.with_prepared_owner(preparation_attempt,|session| {
                    // A cache hit must obey the same owner layout contract as
                    // the ordinary body parser, before any replay can mutate state.
                    let entry_plans=validated_structure_entry_plans(&candidate,tree,body_mode)
                        .map_err(|message|StructureParseError::from(session.environment_error(message)))?;
                    validate_structure_body_layout(body_mode,validator.as_ref(),&candidate.entry_body_modes)
                        .map_err(|message|StructureParseError::from(session.environment_error(message)))?;
                    let mut reused=None;
                    if observed {
                        body_boundary_entered=true;
                        let body_context=session.context().clone();
                        let work=session.body_work();
                        let config=session.body_config();
                        let request=|| crate::StructureBodyReuseRequest {source,tree,candidate:&candidate,context:&body_context,body_mode,work,parent_attempt:preparation_attempt,ambiguous:false,config:config.clone()};
                        let plan=session.environment_mut().prepare_structure_body(request())
                            .map_err(|message|session.environment_error(message))?;
                        session.check_cancelled()?;
                        if let crate::StructureBodyPlan::Reuse(replay)=plan {
                            if session.body_work_is_valid(replay.before,replay.work)
                                && valid_body_preparations(source,tree,node,preparation_attempt,&replay)
                                && session.environment_mut().accept_structure_body_replay(request())
                                    .map_err(|message|session.environment_error(message))? {
                                session.check_cancelled()?;
                                session.apply_body_work(replay.before,replay.work);
                                for preparation in &replay.preparations {
                                    session.check_cancelled()?;
                                    session.publish_preparation(StructurePreparationEvent::SectionHeader {
                                        source,tree,candidate:&preparation.candidate,
                                        attempt_id:preparation.attempt_id,parent_attempt_id:preparation.parent_attempt_id,
                                    });
                                }
                                session.check_cancelled()?;
                                session.replace_context(replay.post_context);
                                reused=Some(ParsedStructureBody {body:replay.body.into_body(),diagnostics:Vec::new(),entries_valid:true});
                            }
                        }
                    }
                    match reused {
                        Some(parsed)=>Ok(parsed),
                        None=>parse_structure_body(session,tree,node,body_mode,validator.as_ref(),&candidate.entry_body_modes,&entry_plans,1),
                    }
                });
                let child_context = session.replace_context(saved_context);
                let mut parsed = parsed?;
                if observed {
                    let work=session.body_work();
                    let release=session.environment_mut().finish_structure_body(&mut parsed.body,&child_context,work)
                        .map_err(|message|session.environment_error(message))?;
                    session.check_cancelled()?;
                    if release && let Some(span)=&node.body_span {
                        session.release_body_resources(span.virtual_range);
                    }
                }
                candidate.body = parsed.body;
                if !parsed.entries_valid {
                    candidate.entry_validation_failed = true;
                    return Ok((
                        candidate,
                        parsed.diagnostics,
                        Some(("Structure entries failed validation".to_owned(), Vec::new())),
                    ));
                }
                let exit = StructureHookRequest {
                    input: session.source().virtual_source(),
                    tree,
                    timing: StructureHookTiming::ExitBody,
                    candidate: &candidate,
                    context: &child_context,
                    default_body_mode: body_mode,
                };
                let rejection = match session
                    .environment_mut()
                    .exit_structure(exit)
                    .map_err(|message| session.environment_error(message))?
                {
                    StructureExitDecision::Accept { metadata } => {
                        candidate.metadata = metadata;
                        None
                    },
                    StructureExitDecision::Reject {
                        reason,
                        diagnostics,
                    } => Some((reason, diagnostics)),
                };
                session.check_cancelled()?;
                Ok((candidate, parsed.diagnostics, rejection))
            })();
            let accepted = attempt
                .as_ref()
                .is_ok_and(|(_, _, rejection)| rejection.is_none());
            let finished = session.finish_semantic_candidate(accepted);
            // Finishing a provider scope may itself observe cancellation. Do
            // not publish a pending body as accepted in that case.
            let cancelled = session.check_cancelled();
            if body_boundary_entered {
                session.environment_mut().adopt_structure_body(accepted && finished.is_ok() && cancelled.is_ok())
                    .map_err(|message|session.environment_error(message))?;
            }
            let adopted = session.check_cancelled();
            if !accepted || finished.is_err() {
                session.publish_preparation(StructurePreparationEvent::Invalidated {
                    attempt_id: preparation_attempt,
                    raw_node_id: provisional_candidate.raw_node_id,
                    registration_id: &provisional_candidate.matched.registration_id,
                    pattern_index: provisional_candidate.matched.pattern_index,
                });
            }
            finished.map_err(|message| session.environment_error(message))?;
            cancelled?;
            adopted?;
            session.replace_context(document_context.clone());
            let (candidate, body_diagnostics, rejection) = match attempt {
                Ok(value) => value,
                Err(error) if crate::function_namespace_unavailable(&error).is_some() => {
                    session
                        .environment_mut()
                        .discard_structure_candidate(&provisional_candidate)
                        .map_err(|message| session.environment_error(message))?;
                    let unavailable = crate::function_namespace_unavailable(&error)
                        .expect("guard checked dependency");
                    let span = session.map_range(node.span.virtual_range)?;
                    session
                        .environment_mut()
                        .observe_function_namespace_unavailable(unavailable, span.mapped);
                    best_rejection = None;
                    matches.unknown = None;
                    matches.selected = None;
                    matches.alternatives.clear();
                    break;
                }
                Err(error) if crate::extension_fault(&error).is_some() => {
                    session
                        .environment_mut()
                        .discard_structure_candidate(&provisional_candidate)
                        .map_err(|message| session.environment_error(message))?;
                    let fault = crate::extension_fault(&error).expect("guard checked fault");
                    let span = session.map_range(
                        node.code_span
                            .as_ref()
                            .map_or(node.span.virtual_range, |span| span.virtual_range),
                    )?;
                    session
                        .environment_mut()
                        .observe_extension_fault(fault, span.mapped);
                    diagnostics.push(node_diagnostic(
                        &session,
                        node,
                        StructureDiagnosticKind::ExtensionFault,
                        &error.to_string(),
                    )?);
                    best_rejection = None;
                    matches.unknown = None;
                    matches.selected = None;
                    matches.alternatives.clear();
                    break;
                }
                Err(error) => return Err(error),
            };
            if let Some((reason, semantic_diagnostics)) = rejection {
                session
                    .environment_mut()
                    .discard_structure_candidate(&candidate)
                    .map_err(|message| session.environment_error(message))?;
                best_rejection.get_or_insert((
                    candidate,
                    body_diagnostics,
                    reason,
                    semantic_diagnostics,
                ));
                continue;
            }
            diagnostics.extend(body_diagnostics);
            matches.selected = Some(candidate);
            matches.alternatives = candidates
                .into_iter()
                .map(|(candidate, _)| candidate)
                .collect();
            break;
        }

        if matches.selected.is_none()
            && let Some((candidate, mut body_diagnostics, reason, semantic_diagnostics)) =
                best_rejection
        {
            let range = structure_header_range(session.source(), node)?;
            let source = range
                .slice(session.source().virtual_source())
                .ok_or(StructureParseError::InvalidRange { range })?
                .to_owned();
            let matched = candidate.matched.clone();
            matches.unknown = Some(UnknownStructureNode {
                raw_node_id: node.id,
                source,
                span: session.map_range(range)?,
                failure: Some(structure_semantic_rejection(
                    &matched,
                    reason,
                    semantic_diagnostics,
                )),
                partial: Some(Box::new(candidate)),
            });
            let diagnostic = node_diagnostic(
                &session,
                node,
                StructureDiagnosticKind::Unclaimed,
                "Structure body was not accepted",
            )?;
            matches.diagnostics.push(diagnostic.clone());
            diagnostics.push(diagnostic);
            diagnostics.append(&mut body_diagnostics);
        }
    }

    Ok(StructureDocument { roots, diagnostics })
}

fn valid_body_preparations(
    source:&MappedSource,
    tree:&RawTree,
    root:&RawNode,
    parent:PreparationAttemptId,
    replay:&crate::StructureBodyReplay,
) -> bool {
    // Preparations are the actual enter-time proposals, while the portable AST
    // contains exit-time results. Their owner identities and traversal must
    // agree one-to-one; final context/metadata must not replace the proposals.
    if !replay.body.matches_preparations(&replay.preparations) {return false;}
    let Some(body)=&root.body_span else { return replay.preparations.is_empty(); };
    let mut seen=BTreeMap::new();
    for preparation in &replay.preparations {
        let Some(node)=tree.get(preparation.candidate.raw_node_id) else { return false; };
        let Some(parent_node)=preparation.parent_attempt_id.and_then(|id|
            if id==parent {Some(root.id)}else{seen.get(&id).copied()}) else { return false; };
        let Ok(header)=structure_header_range(source,node) else { return false; };
        let Ok(mapped)=source.map_range(header) else { return false; };
        if node.kind!=RawNodeKind::Section || node.span.virtual_range.start<body.virtual_range.start
            || node.span.virtual_range.end>body.virtual_range.end
            || node.parent!=Some(parent_node)
            || preparation.candidate.matched.matched.span.mapped!=mapped
            || preparation.attempt_id<replay.before.next_preparation_attempt
            || preparation.attempt_id>=replay.work.next_preparation_attempt
            || !preparation.candidate.body.is_empty()
            || seen.insert(preparation.attempt_id,node.id).is_some() { return false; }
    }
    true
}

fn structure_semantic_rejection(
    matched: &CandidateMatch,
    reason: String,
    diagnostics: Vec<crate::SemanticDiagnostic>,
) -> FailureTrace {
    let candidate_span = matched.matched.span.clone();
    let span = crate::failure::semantic_failure_span(&candidate_span, &diagnostics);
    FailureTrace::leaf(PatternFailure {
        span: span.clone(),
        reasons: vec![PatternFailureReason::HookRejected { reason }],
    })
    .with_parent(FailureFrame {
        kind: matched.kind,
        definition_id: matched.definition_id.clone(),
        registration_id: matched.registration_id.clone(),
        pattern_index: matched.pattern_index,
        pattern: matched.pattern.clone(),
        element_path: Vec::new(),
        pattern_span: None,
        input_span: candidate_span,
        role: FailureFrameRole::SemanticCandidate,
    })
    .with_semantic_diagnostics(diagnostics)
}

fn parse_structure_header<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    tree: &RawTree,
    node: &RawNode,
) -> Result<
    (
        StructureMatches,
        Option<ExpressionParseContext>,
        Option<StructureBodyMode>,
    ),
    StructureParseError,
> {
    let range = structure_header_range(session.source(), node)?;
    let actual_node_type = match node.kind {
        RawNodeKind::Simple => NodeType::Simple,
        RawNodeKind::Section => NodeType::Section,
        _ => unreachable!("caller filters Structure node kinds"),
    };
    let mut candidates = session.syntax_candidates(SyntaxKind::Structure);
    candidates.retain(|candidate| {
        declared_node_type(session, &candidate.registration_id)
            .is_none_or(|declared| accepts_node_type(declared, actual_node_type))
    });
    session.retain_viable_patterns(range, &mut candidates)?;
    let matched = session.match_candidates_at_depth(range, &candidates, 0)?;
    let mut failure = matched.primary_failure().cloned();
    let mut ranked = matched
        .selected
        .into_iter()
        .chain(matched.alternatives)
        .collect::<Vec<_>>();
    let mut accepted = Vec::new();
    let initial_context = session.context().clone();
    let mut selected_context = None;
    let mut selected_body_mode = None;

    for matched in ranked.drain(..) {
        session.check_cancelled()?;
        session.replace_context(initial_context.clone());
        session
            .begin_semantic_candidate()
            .map_err(|message| session.environment_error(message))?;
        let selected_candidate = accepted.is_empty();
        let attempt: Result<_, StructureParseError> = (|| {
            session
                .activate_pattern_candidate(&matched)
                .map_err(|message| session.environment_error(message))?;
            let mut candidate =
                structure_candidate(session, node.id, actual_node_type, matched, range.start, 0)?;
            let mut accepted_semantically = false;
            if let Some(candidate) = candidate.as_mut() {
                let default_body_mode = default_body_mode(session, candidate);
                let context = session.context().clone();
                let request = StructureHookRequest {
                    input: session.source().virtual_source(),
                    tree,
                    timing: StructureHookTiming::EnterBody,
                    candidate,
                    context: &context,
                    default_body_mode,
                };
                match session
                    .environment_mut()
                    .enter_structure(request)
                    .map_err(|message| session.environment_error(message))?
                {
                    StructureHookDecision::Accept {
                        context,
                        body_mode,
                        entry_body_modes,
                        metadata,
                    } => {
                        accepted_semantically = true;
                        if selected_candidate {
                            // A bodyless Structure is a document directive. Its context
                            // changes apply to following roots; body-bearing Structures
                            // keep their context scoped to their own body.
                            if body_mode == StructureBodyMode::None {
                                session.replace_context(context.clone());
                            }
                            selected_context = Some(context);
                            selected_body_mode = Some(body_mode);
                        }
                        candidate.entry_body_modes = entry_body_modes;
                        candidate.metadata = metadata;
                    }
                    StructureHookDecision::Reject {
                        reason,
                        diagnostics,
                    } => {
                        failure = crate::choose_failure_trace(
                            failure.take(),
                            Some(structure_semantic_rejection(
                                &candidate.matched,
                                reason,
                                diagnostics,
                            )),
                        );
                    }
                }
            }
            Ok((candidate, accepted_semantically))
        })();
        let keep = attempt
            .as_ref()
            .is_ok_and(|(_, accepted)| selected_candidate && *accepted);
        session
            .finish_semantic_candidate(keep)
            .map_err(|message| session.environment_error(message))?;
        let (candidate, accepted_semantically) = attempt?;
        if let Some(candidate) = candidate
            && accepted_semantically
        {
            accepted.push(candidate);
        }
    }

    if selected_body_mode == Some(StructureBodyMode::None) {
        session.replace_context(selected_context.clone().unwrap_or(initial_context));
    } else {
        session.replace_context(initial_context);
    }

    let mut diagnostics = Vec::new();
    let mut selected = (!accepted.is_empty()).then(|| accepted.remove(0));
    let alternatives = accepted;
    if selected.is_some() && !alternatives.is_empty() {
        diagnostics.push(node_diagnostic(
            session,
            node,
            StructureDiagnosticKind::MultipleClaims,
            "multiple Structure registrations accept this header",
        )?);
    }
    let unknown = if selected.is_none() {
        diagnostics.push(node_diagnostic(
            session,
            node,
            StructureDiagnosticKind::Unclaimed,
            "top-level node is not claimed by a Structure",
        )?);
        let source = range
            .slice(session.source().virtual_source())
            .ok_or(StructureParseError::InvalidRange { range })?
            .to_owned();
        Some(UnknownStructureNode {
            raw_node_id: node.id,
            source,
            span: session.map_range(range)?,
            failure,
            partial: None,
        })
    } else {
        None
    };
    if let Some(candidate) = selected.as_mut() {
        candidate.body = StructureBody::None;
    }
    Ok((
        StructureMatches {
            selected,
            alternatives,
            unknown,
            diagnostics,
        },
        selected_context,
        selected_body_mode,
    ))
}

fn structure_candidate<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    raw_node_id: RawNodeId,
    actual_node_type: NodeType,
    matched: CandidateMatch,
    frame_start: usize,
    depth: usize,
) -> Result<Option<StructureCandidate>, StructureParseError> {
    let structure = session
        .catalog()
        .structures()
        .find(|structure| structure.common.registration_id.as_str() == matched.registration_id);
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    let element_class = structure.map(|value| value.common.element_class.clone());
    let declared_node_type = structure
        .and_then(|value| value.node_type)
        .or_else(|| dynamic.and_then(|value| value.structure_node_type))
        .unwrap_or(NodeType::Both);
    let dynamic_handler = dynamic.map(|definition| definition.handler.as_str());
    let capture_bindings = session
        .environment()
        .registered_capture_bindings(RegisteredSyntaxIdentity {
            kind: SyntaxKind::Structure,
            definition_id: &matched.definition_id,
            registration_id: &matched.registration_id,
            pattern_index: Some(matched.pattern_index),
            pattern_source: Some(&matched.pattern),
            tags: Some(&matched.matched.tags),
            mark: Some(matched.matched.mark),
            dynamic_handler,
        })
        .map_err(|message| session.environment_error(message))?;
    let mut parsed_captures = Vec::new();
    for capture in &matched.matched.captures {
        let capture_index = capture.capture_index();
        if let PatternCapture::TypeExpression {
            resolution_id: Some(id),
            ..
        } = capture
            && let Some(node) = session.resolved_node(id).cloned()
        {
            parsed_captures.push(crate::expression_parsed_capture(capture_index, node));
            continue;
        }
        let PatternCapture::Regex { span, .. } = capture else {
            continue;
        };
        let Some(binding) = capture_bindings
            .iter()
            .find(|binding| binding.capture_index == capture_index)
        else {
            continue;
        };
        let local = span.local_range;
        let range = TextRange::new(frame_start + local.start, frame_start + local.end);
        let parsed = match binding.parser_id.as_str() {
            HOST_EVENT_PARSER_ID => {
                let parsed =
                    crate::event::parse_event_range_with_session(session, range, depth + 1)
                        .map_err(nested_structure_error)?;
                match parsed.selected {
                    Some(selected) => {
                        crate::event_parsed_capture(capture_index, binding.clone(), selected)
                    }
                    None if binding.required => return Ok(None),
                    None => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::failure(
                            binding.parser_id.clone(),
                            span.clone(),
                            "event capture did not match",
                        ),
                    },
                }
            }
            HOST_CONDITION_PARSER_ID => {
                let context = session
                    .capture_context(binding)
                    .map_err(|message| session.environment_error(message))?;
                let previous = context.map(|context| session.replace_context(context));
                let parsed =
                    crate::condition::parse_condition_with_session(session, range, depth + 1);
                if let Some(previous) = previous {
                    session.replace_context(previous);
                }
                let parsed = parsed.map_err(nested_structure_error)?;
                match parsed.selected {
                    Some(selected) => {
                        let mut capture =
                            crate::condition_parsed_capture(capture_index, selected.node);
                        capture.binding = binding.clone();
                        capture
                    }
                    None if binding.required => return Ok(None),
                    None => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::failure(
                            binding.parser_id.clone(),
                            span.clone(),
                            "condition capture did not match",
                        ),
                    },
                }
            }
            HOST_EFFECT_PARSER_ID => {
                let context = session
                    .capture_context(binding)
                    .map_err(|message| session.environment_error(message))?;
                let previous = context.map(|context| session.replace_context(context));
                let parsed = crate::effect::parse_effect_range_with_session(
                    session,
                    range,
                    raw_node_id,
                    depth + 1,
                );
                if let Some(previous) = previous {
                    session.replace_context(previous);
                }
                let parsed = parsed.map_err(nested_structure_error)?;
                match parsed.into_selected_capture(session.catalog()) {
                    Ok((summary, value)) => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::success(
                            binding.parser_id.clone(),
                            span.clone(),
                            Some(summary),
                            value,
                        ),
                    },
                    Err(_) if binding.required => return Ok(None),
                    Err(_) => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::failure(
                            binding.parser_id.clone(),
                            span.clone(),
                            "effect capture did not match",
                        ),
                    },
                }
            }
            _ if binding.required => return Ok(None),
            _ => ParsedCapture {
                capture_index,
                binding: binding.clone(),
                result: ParsedCaptureResult::failure(
                    binding.parser_id.clone(),
                    span.clone(),
                    format!("no native parser route for {}", binding.parser_id),
                ),
            },
        };
        if parsed.result.status != ParsedCaptureStatus::Failed || !binding.required {
            parsed_captures.push(parsed);
        }
    }
    let entry_validator = structure_entry_validator(session, &matched.registration_id);
    Ok(Some(StructureCandidate {
        raw_node_id,
        matched,
        element_class,
        declared_node_type,
        actual_node_type,
        parsed_captures,
        body: StructureBody::None,
        entry_validation_failed: false,
        entry_validator,
        entry_body_modes: BTreeMap::new(),
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
    }))
}

fn default_body_mode<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    candidate: &StructureCandidate,
) -> StructureBodyMode {
    if candidate.actual_node_type == NodeType::Simple {
        return StructureBodyMode::None;
    }
    if let Some(dynamic) = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == candidate.matched.registration_id)
    }) {
        if let Some(mode) = dynamic.structure_body_mode {
            return dynamic_body_mode(mode);
        }
        return dynamic
            .entry_validator
            .as_ref()
            .map_or(StructureBodyMode::Raw, |_| StructureBodyMode::Entries);
    }
    session
        .catalog()
        .structures()
        .find(|structure| {
            structure.common.registration_id.as_str() == candidate.matched.registration_id
        })
        .and_then(|structure| structure.entry_validator.as_ref())
        .map_or(StructureBodyMode::Raw, |_| StructureBodyMode::Entries)
}

/// Validates the owner-declared layout independently of body evaluation.
pub fn validate_structure_body_layout(
    mode: StructureBodyMode,
    validator: Option<&EntryValidator>,
    entry_body_modes: &BTreeMap<Vec<String>, crate::SectionBodyMode>,
) -> Result<(), String> {
    for (path, entry_mode) in entry_body_modes {
        let mut current = validator;
        let mut declaration = None;
        for key in path {
            declaration = current.and_then(|v| v.entry_data.iter().find(|d| d.key == *key));
            current = declaration
                .filter(|d| d.kind == EntryKind::Container)
                .and_then(|d| d.nested_validator.as_ref());
        }
        if mode != StructureBodyMode::Entries
            || path.is_empty()
            || declaration.is_none_or(|d| {
                d.kind != EntryKind::Section
                    && !(d.kind == EntryKind::Trigger
                        && *entry_mode == crate::SectionBodyMode::Trigger)
            })
        {
            return Err(format!("invalid semantic Section entry path {path:?}"));
        }
    }
    Ok(())
}

fn parse_structure_body<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    tree: &RawTree,
    node: &RawNode,
    mode: StructureBodyMode,
    validator: Option<&EntryValidator>,
    entry_body_modes: &BTreeMap<Vec<String>, crate::SectionBodyMode>,
    entry_plans: &StructureEntryPlans,
    depth: usize,
) -> Result<ParsedStructureBody, StructureParseError> {
    let _measurement = session.environment().measurement_scope("native.structure_body");
    validate_structure_body_layout(mode, validator, entry_body_modes)
        .map_err(|message| StructureParseError::from(session.environment_error(message)))?;
    match mode {
        StructureBodyMode::None => Ok(ParsedStructureBody {
            body: StructureBody::None,
            diagnostics: Vec::new(),
            entries_valid: true,
        }),
        StructureBodyMode::Raw => Ok(ParsedStructureBody {
            body: StructureBody::Raw(node.children.clone()),
            diagnostics: Vec::new(),
            entries_valid: true,
        }),
        StructureBodyMode::Trigger => {
            let (body, diagnostics) = crate::section::parse_section_body(
                session,
                tree,
                node,
                crate::SectionBodyMode::Trigger,
                depth,
            )
            .map_err(nested_structure_error)?;
            Ok(ParsedStructureBody {
                body: StructureBody::Trigger(body),
                diagnostics: diagnostics
                    .into_iter()
                    .map(section_diagnostic_to_structure)
                    .collect(),
                entries_valid: true,
            })
        }
        StructureBodyMode::Entries => {
            let Some(validator) = validator else {
                return Ok(ParsedStructureBody {
                    body: StructureBody::Raw(node.children.clone()),
                    diagnostics: Vec::new(),
                    entries_valid: true,
                });
            };
            let (entries, diagnostics, valid) = parse_validator_entries(
                session,
                tree,
                node,
                validator,
                entry_body_modes,
                entry_plans,
                &[],
                depth,
            )?;
            Ok(ParsedStructureBody {
                body: StructureBody::Entries(entries),
                diagnostics,
                entries_valid: valid,
            })
        }
    }
}

fn parse_validator_entries<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    tree: &RawTree,
    parent: &RawNode,
    validator: &EntryValidator,
    entry_body_modes: &BTreeMap<Vec<String>, crate::SectionBodyMode>,
    entry_plans: &StructureEntryPlans,
    path: &[String],
    depth: usize,
) -> Result<(Vec<StructureEntry>, Vec<StructureDiagnostic>, bool), StructureParseError> {
    let mut entries = Vec::new();
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    let mut valid = true;
    for child_id in &parent.children {
        session.check_cancelled()?;
        let Some(child) = tree.get(*child_id) else {
            continue;
        };
        if matches!(child.kind, RawNodeKind::Blank | RawNodeKind::Comment) {
            continue;
        }
        let matched = if path.is_empty() {structure_entry_index_with_plans(validator,child,&seen,entry_plans)}
            else {structure_entry_index(validator,child,&seen)}
            .map(|index| (index, &validator.entry_data[index]));
        let Some((index, data)) = matched else {
            valid = false;
            let duplicate = validator
                .entry_data
                .iter()
                .enumerate()
                .find(|(index, data)| {
                    !data.multiple && seen.contains(index) && structure_entry_matches(data, child)
                });
            diagnostics.push(if let Some((_, data)) = duplicate {
                node_diagnostic(
                    session,
                    child,
                    StructureDiagnosticKind::DuplicateEntry,
                    format!("entry {:?} may only occur once", data.key),
                )?
            } else {
                node_diagnostic(
                    session,
                    child,
                    StructureDiagnosticKind::Unclaimed,
                    "entry is not accepted by this Structure",
                )?
            });
            continue;
        };
        seen.insert(index);
        let mut entry_path = path.to_vec();
        entry_path.push(data.key.clone());
        let (entry, diagnostic, entry_valid) = parse_entry(
            session,
            tree,
            child,
            index,
            data,
            entry_body_modes,
            if path.is_empty() {entry_plans.entries.get(child_id)} else {None},
            &entry_path,
            depth,
        )?;
        entries.push(entry);
        diagnostics.extend(diagnostic);
        valid &= entry_valid;
    }
    for (index, data) in validator.entry_data.iter().enumerate() {
        if seen.contains(&index) {
            continue;
        }
        if let Some(default) = data.default_value.as_ref() {
            let source = default
                .as_str()
                .map_or_else(|| default.to_string(), str::to_owned);
            entries.push(StructureEntry {
                raw_node_id: None,
                declaration_index: index,
                key_span: None,
                key: data.key.clone(),
                entry_data_class: data.entry_data_class.clone(),
                kind: data.kind.clone(),
                source: source.clone(),
                span: session.map_range(TextRange::empty(parent.span.virtual_range.end))?,
                defaulted: true,
                value: StructureEntryValue::Raw(source),
            });
        }
        if !data.optional {
            valid = false;
            let missing_at = parent
                .body_span
                .as_ref()
                .map_or(parent.span.virtual_range.end, |span| span.virtual_range.end);
            diagnostics.push(StructureDiagnostic {
                raw_node_id: Some(parent.id),
                kind: StructureDiagnosticKind::MissingRequiredEntry,
                span: session.map_range(TextRange::empty(missing_at))?,
                message: format!("required Structure entry {:?} is missing", data.key),
            });
        }
    }
    Ok((entries, diagnostics, valid))
}

fn parse_entry<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    tree: &RawTree,
    node: &RawNode,
    declaration_index: usize,
    data: &EntryData,
    entry_body_modes: &BTreeMap<Vec<String>, crate::SectionBodyMode>,
    entry_plan: Option<&StructureEntryPlan>,
    path: &[String],
    depth: usize,
) -> Result<(StructureEntry, Vec<StructureDiagnostic>, bool), StructureParseError> {
    let source = node.text.clone();
    let span = session.map_range(
        node.code_span
            .as_ref()
            .map_or(node.span.virtual_range, |span| span.virtual_range),
    )?;
    let start = node
        .code_span
        .as_ref()
        .map_or(node.span.virtual_range.start, |span| {
            span.virtual_range.start
        });
    let key_range = TextRange::new(start, start + data.key.len());
    let key_span = key_range
        .slice(session.source().virtual_source())
        .filter(|text| text.eq_ignore_ascii_case(&data.key))
        .map(|_| session.map_range(key_range))
        .transpose()?;
    let value_range = entry_value_range(node, data)?;
    let raw_value = value_range
        .and_then(|range| range.slice(session.source().virtual_source()))
        .unwrap_or_default()
        .to_owned();
    let mut diagnostics = Vec::new();
    let mut valid = true;
    let value = if let Some(plan)=entry_plan {
        let context=plan.context.apply(session.context());
        let previous=session.replace_context(context);
        let parsed=crate::section::parse_section_body(session,tree,node,plan.mode,depth);
        session.replace_context(previous);
        let (body,section_diagnostics)=parsed.map_err(nested_structure_error)?;
        diagnostics.extend(section_diagnostics.into_iter().map(section_diagnostic_to_structure));
        StructureEntryValue::Trigger(body)
    } else {match data.kind {
        EntryKind::Literal | EntryKind::Expression => {
            let expected = if data.kind == EntryKind::Literal {
                data.value_type.iter().cloned().collect::<Vec<_>>()
            } else {
                data.return_types.clone()
            }
            .into_iter()
            .map(|class_name| ExpressionExpectedType {
                class_name,
                plural: false,
            })
            .collect::<Vec<_>>();
            let flags = data.flags.unwrap_or(3);
            let allow_literals = data.kind == EntryKind::Literal || flags & 2 != 0;
            let allow_expressions = data.kind == EntryKind::Expression && flags & 1 != 0;
            let parsed = value_range
                .map(|range| {
                    session.parse_complete_range(
                        range,
                        &expected,
                        allow_literals,
                        allow_expressions,
                        depth,
                    )
                })
                .transpose()?;
            match parsed.and_then(|matches| matches.selected) {
                Some(selected) => StructureEntryValue::Expression(Box::new(selected.node)),
                None => {
                    valid = false;
                    diagnostics.push(node_diagnostic(
                        session,
                        node,
                        StructureDiagnosticKind::InvalidEntryValue,
                        format!("entry {:?} has an invalid value", data.key),
                    )?);
                    StructureEntryValue::Raw(raw_value)
                }
            }
        }
        EntryKind::Trigger => {
            let (body, section_diagnostics) = crate::section::parse_section_body(
                session,
                tree,
                node,
                crate::SectionBodyMode::Trigger,
                depth,
            )
            .map_err(nested_structure_error)?;
            diagnostics.extend(
                section_diagnostics
                    .into_iter()
                    .map(section_diagnostic_to_structure),
            );
            StructureEntryValue::Trigger(body)
        }
        EntryKind::Container => {
            let Some(nested) = data.nested_validator.as_ref() else {
                diagnostics.push(node_diagnostic(
                    session,
                    node,
                    StructureDiagnosticKind::InvalidEntryValue,
                    format!("container entry {:?} has no nested validator", data.key),
                )?);
                valid = false;
                return Ok((
                    structure_entry(
                        node,
                        declaration_index,
                        key_span,
                        data,
                        source,
                        span,
                        StructureEntryValue::Raw(raw_value),
                    ),
                    diagnostics,
                    valid,
                ));
            };
            let (children, nested_diagnostics, nested_valid) = parse_validator_entries(
                session,
                tree,
                node,
                nested,
                entry_body_modes,
                &StructureEntryPlans::default(),
                path,
                depth + 1,
            )?;
            diagnostics.extend(nested_diagnostics);
            valid &= nested_valid;
            StructureEntryValue::Container(children)
        }
        EntryKind::Section => {
            if let Some(mode) = entry_body_modes.get(path) {
                let (body, section_diagnostics) =
                    crate::section::parse_section_body(session, tree, node, *mode, depth)
                        .map_err(nested_structure_error)?;
                diagnostics.extend(
                    section_diagnostics
                        .into_iter()
                        .map(section_diagnostic_to_structure),
                );
                StructureEntryValue::Trigger(body)
            } else {
                StructureEntryValue::Section(node.children.clone())
            }
        }
        EntryKind::Unknown => {
            diagnostics.push(node_diagnostic(
                session,
                node,
                StructureDiagnosticKind::UnknownEntryData,
                format!(
                    "entry {:?} uses addon-defined EntryData {}",
                    data.key, data.entry_data_class.0
                ),
            )?);
            StructureEntryValue::Unknown(raw_value)
        }
        EntryKind::VariableString | EntryKind::KeyValue => StructureEntryValue::Raw(raw_value),
    }};
    Ok((
        structure_entry(node, declaration_index, key_span, data, source, span, value),
        diagnostics,
        valid,
    ))
}

fn structure_entry(
    node: &RawNode,
    declaration_index: usize,
    key_span: Option<MatchSpan>,
    data: &EntryData,
    source: String,
    span: MatchSpan,
    value: StructureEntryValue,
) -> StructureEntry {
    StructureEntry {
        raw_node_id: Some(node.id),
        declaration_index,
        key_span,
        key: data.key.clone(),
        entry_data_class: data.entry_data_class.clone(),
        kind: data.kind.clone(),
        source,
        span,
        defaulted: false,
        value,
    }
}

/// Tests an EntryValidator declaration against a physical source node.
/// Selects the next physical entry using the native ordered declaration contract.
pub fn structure_entry_index(
    validator: &EntryValidator,
    node: &RawNode,
    seen: &BTreeSet<usize>,
) -> Option<usize> {
    validator
        .entry_data
        .iter()
        .enumerate()
        .filter(|(index, data)| data.multiple || !seen.contains(index))
        .find(|(_, data)| structure_entry_matches(data, node))
        .map(|(index, _)| index)
}

pub fn structure_entry_matches(data: &EntryData, node: &RawNode) -> bool {
    match data.kind {
        EntryKind::Trigger | EntryKind::Container | EntryKind::Section => {
            node.kind == RawNodeKind::Section && node.text.eq_ignore_ascii_case(&data.key)
        }
        EntryKind::Unknown if node.kind == RawNodeKind::Section => {
            node.text.eq_ignore_ascii_case(&data.key)
        }
        _ => {
            if node.kind != RawNodeKind::Simple {
                return false;
            }
            let separator = data.separator.as_deref().unwrap_or(": ");
            let Some(rest) = strip_prefix_ignore_ascii_case(&node.text, &data.key) else {
                return false;
            };
            rest.starts_with(separator)
        }
    }
}

fn entry_value_range(
    node: &RawNode,
    data: &EntryData,
) -> Result<Option<TextRange>, StructureParseError> {
    if matches!(
        data.kind,
        EntryKind::Trigger | EntryKind::Container | EntryKind::Section
    ) || data.kind == EntryKind::Unknown && node.kind == RawNodeKind::Section
    {
        return Ok(None);
    }
    let code = node
        .code_span
        .as_ref()
        .ok_or(StructureParseError::MissingCodeSpan { node_id: node.id })?
        .virtual_range;
    let separator = data.separator.as_deref().unwrap_or(": ");
    let offset = data.key.len() + separator.len();
    Ok(Some(TextRange::new(code.start + offset, code.end)))
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

fn declared_node_type<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    registration_id: &str,
) -> Option<NodeType> {
    session
        .catalog()
        .structures()
        .find(|structure| structure.common.registration_id.as_str() == registration_id)
        .map(|structure| structure.node_type.unwrap_or(NodeType::Both))
        .or_else(|| {
            session
                .dynamic_snapshot()
                .and_then(|snapshot| {
                    snapshot
                        .definitions
                        .values()
                        .find(|definition| definition.id.qualified() == registration_id)
                })
                .map(|definition| definition.structure_node_type.unwrap_or(NodeType::Both))
        })
}

/// Addon decision after the selected Structure body has been parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureExitDecision {
    /// Adopt validated candidate metadata only after successful Exit.
    Accept {
        /// Includes host provenance from the final ordered Exit hooks.
        metadata: BTreeMap<String, String>,
    },
    Reject {
        reason: String,
        diagnostics: Vec<crate::SemanticDiagnostic>,
    },
}

fn structure_entry_validator<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    registration_id: &str,
) -> Option<EntryValidator> {
    session
        .catalog()
        .structures()
        .find(|structure| structure.common.registration_id.as_str() == registration_id)
        .and_then(|structure| structure.entry_validator.clone())
        .or_else(|| {
            session
                .dynamic_snapshot()
                .and_then(|snapshot| {
                    snapshot
                        .definitions
                        .values()
                        .find(|definition| definition.id.qualified() == registration_id)
                })
                .and_then(|definition| definition.entry_validator.clone())
        })
}

fn dynamic_body_mode(value: syntaxes::DynamicStructureBodyMode) -> StructureBodyMode {
    match value {
        syntaxes::DynamicStructureBodyMode::None => StructureBodyMode::None,
        syntaxes::DynamicStructureBodyMode::Raw => StructureBodyMode::Raw,
        syntaxes::DynamicStructureBodyMode::Entries => StructureBodyMode::Entries,
        syntaxes::DynamicStructureBodyMode::Trigger => StructureBodyMode::Trigger,
    }
}

fn accepts_node_type(declared: NodeType, actual: NodeType) -> bool {
    declared == NodeType::Both || declared == actual
}

fn structure_header_range(
    source: &MappedSource,
    node: &RawNode,
) -> Result<TextRange, StructureParseError> {
    let range = node
        .code_span
        .as_ref()
        .ok_or(StructureParseError::MissingCodeSpan { node_id: node.id })?
        .virtual_range;
    let text = range
        .slice(source.virtual_source())
        .ok_or(StructureParseError::InvalidRange { range })?;
    let header = if node.kind == RawNodeKind::Section {
        text.strip_suffix(':')
            .ok_or(StructureParseError::MissingHeaderColon)?
    } else {
        text
    };
    let trimmed = crate::pattern_match::java_trim_range(header);
    Ok(TextRange::new(
        range.start + trimmed.start,
        range.start + trimmed.end,
    ))
}

fn node_diagnostic<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    node: &RawNode,
    kind: StructureDiagnosticKind,
    message: impl Into<String>,
) -> Result<StructureDiagnostic, StructureParseError> {
    let range = node
        .code_span
        .as_ref()
        .map_or(node.span.virtual_range, |span| span.virtual_range);
    Ok(StructureDiagnostic {
        raw_node_id: Some(node.id),
        kind,
        span: session.map_range(range)?,
        message: message.into(),
    })
}

fn section_diagnostic_to_structure(value: SectionDiagnostic) -> StructureDiagnostic {
    StructureDiagnostic {
        raw_node_id: Some(value.raw_node_id),
        kind: match value.kind {
            crate::SectionDiagnosticKind::ExtensionFault => StructureDiagnosticKind::ExtensionFault,
            crate::SectionDiagnosticKind::Unclaimed => StructureDiagnosticKind::Unclaimed,
            crate::SectionDiagnosticKind::MultipleClaims => StructureDiagnosticKind::MultipleClaims,
        },
        span: value.span,
        message: "Structure trigger child could not be parsed uniquely".to_owned(),
    }
}

fn nested_structure_error(error: impl std::error::Error + 'static) -> StructureParseError {
    if let Some(cancelled) = crate::parse_cancellation(&error) {
        return ExpressionParseError::from(*cancelled).into();
    }
    if let Some(unavailable) = crate::function_namespace_unavailable(&error) {
        return ExpressionParseError::from(unavailable.clone()).into();
    }
    match crate::extension_fault(&error).cloned() {
        Some(fault) => ExpressionParseError::ExtensionFault(fault).into(),
        None => StructureParseError::Body {
            message: error.to_string(),
        },
    }
}

#[cfg(test)]
#[test]
fn nested_body_cancellation_preserves_its_typed_cause() {
    let error = crate::SectionParseError::Expression(crate::ParseCancelled.into());
    let error = nested_structure_error(error);
    assert!(crate::parse_cancellation(&error).is_some());
    assert!(crate::extension_fault(&error).is_none());
}
