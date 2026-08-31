//! Structured provenance for failures produced by nested syntax parsing.

use std::cmp::Ordering;

use crate::{
    MappedSpan, MatchSpan, MatchSyntaxKind, PatternFailure, PatternPathSegment, TextRange,
};
use syntax_pattern_parser::syntax::Span as PatternSpan;

/// Semantic role of one syntax frame in a nested failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureFrameRole {
    /// A structurally matched syntax candidate was rejected by semantic hooks.
    SemanticCandidate,
    /// A typed pattern capture failed during recursive Expression parsing.
    TypeExpressionCapture,
    /// A regex capture was reinterpreted as an Expression.
    ExpressionCapture { index: usize },
    /// A regex capture was reinterpreted as a Condition.
    ConditionCapture { index: usize },
    /// A regex capture was reinterpreted as an Effect.
    EffectCapture { index: usize },
}

/// Ranked candidate failures with an optional aggregate matcher fallback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RankedFailures<T> {
    pub fallback: Option<FailureTrace>,
    pub candidates: Vec<T>,
}

impl<T> RankedFailures<T> {
    /// Returns the highest-ranked candidate failure.
    pub fn primary(&self) -> Option<&T> {
        self.candidates.first()
    }
}

/// Syntax and capture context surrounding one parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureFrame {
    pub kind: MatchSyntaxKind,
    pub definition_id: String,
    pub registration_id: String,
    pub pattern_index: usize,
    pub pattern: String,
    pub element_path: Vec<PatternPathSegment>,
    pub pattern_span: Option<PatternSpan>,
    pub input_span: MatchSpan,
    pub role: FailureFrameRole,
}

/// Severity of a semantic diagnostic attached to one rejected candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Secondary source location that explains a semantic diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRelatedSpan {
    pub message: String,
    pub span: MappedSpan,
}

/// Addon- or CoreLibrary-provided detail owned by one candidate failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiagnostic {
    pub code: String,
    pub message: String,
    pub severity: SemanticDiagnosticSeverity,
    pub span: MappedSpan,
    pub related: Vec<SemanticRelatedSpan>,
}

pub(crate) fn semantic_failure_span(
    fallback: &MatchSpan,
    diagnostics: &[SemanticDiagnostic],
) -> MatchSpan {
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.severity == SemanticDiagnosticSeverity::Error)
        .or_else(|| diagnostics.first());
    let Some(diagnostic) = diagnostic else {
        return fallback.clone();
    };
    let virtual_range = diagnostic.span.virtual_range;
    let fallback_virtual = fallback.mapped.virtual_range;
    if virtual_range.start < fallback_virtual.start || virtual_range.end > fallback_virtual.end {
        return fallback.clone();
    }
    MatchSpan {
        local_range: TextRange::new(
            fallback.local_range.start + virtual_range.start - fallback_virtual.start,
            fallback.local_range.start + virtual_range.end - fallback_virtual.start,
        ),
        mapped: diagnostic.span.clone(),
    }
}

/// One failure with its parent syntax and optional more specific cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureTrace {
    pub failure: PatternFailure,
    pub frame: Option<FailureFrame>,
    pub cause: Option<Box<FailureTrace>>,
    pub semantic_diagnostics: Vec<SemanticDiagnostic>,
}

impl FailureTrace {
    /// Creates a leaf trace without surrounding syntax context.
    pub fn leaf(failure: PatternFailure) -> Self {
        Self {
            failure,
            frame: None,
            cause: None,
            semantic_diagnostics: Vec::new(),
        }
    }

    /// Attaches semantic diagnostics to this exact failure frame.
    pub fn with_semantic_diagnostics(mut self, diagnostics: Vec<SemanticDiagnostic>) -> Self {
        self.semantic_diagnostics = diagnostics;
        self
    }

    /// Returns diagnostics from the deepest cause through its parent frames.
    pub fn semantic_diagnostics(&self) -> Vec<&SemanticDiagnostic> {
        let mut diagnostics = self
            .cause
            .as_deref()
            .map(FailureTrace::semantic_diagnostics)
            .unwrap_or_default();
        diagnostics.extend(self.semantic_diagnostics.iter());
        diagnostics
    }

    /// Returns the deepest available cause, which is normally the best primary label.
    pub fn root_cause(&self) -> &Self {
        self.cause.as_deref().map_or(self, FailureTrace::root_cause)
    }

    /// Prepends a syntax frame while retaining the original failure summary.
    pub fn with_parent(self, frame: FailureFrame) -> Self {
        Self {
            failure: self.failure.clone(),
            frame: Some(frame),
            cause: Some(Box::new(self)),
            semantic_diagnostics: Vec::new(),
        }
    }

    pub(crate) fn specificity(&self) -> usize {
        let mut ranges = Vec::new();
        let mut current = Some(self);
        while let Some(trace) = current {
            if let Some(frame) = &trace.frame {
                let range = frame.input_span.mapped.virtual_range;
                if ranges.last() != Some(&range) {
                    ranges.push(range);
                }
            }
            current = trace.cause.as_deref();
        }
        let root = self.root_cause().failure.span.mapped.virtual_range;
        if ranges.last() != Some(&root) {
            ranges.push(root);
        }
        ranges.len()
    }

    fn is_event_restriction(&self) -> bool {
        self.root_cause()
            .failure
            .reasons
            .iter()
            .any(|reason| matches!(reason, crate::PatternFailureReason::EventRestricted { .. }))
    }

    fn is_semantic_rejection(&self) -> bool {
        self.frame
            .as_ref()
            .is_some_and(|frame| frame.role == FailureFrameRole::SemanticCandidate)
            || self
                .cause
                .as_deref()
                .is_some_and(FailureTrace::is_semantic_rejection)
    }
}

pub(crate) fn compare_failure_rank(
    current_range: TextRange,
    current_specificity: usize,
    candidate_range: TextRange,
    candidate_specificity: usize,
) -> Ordering {
    compare_failure_rank_at_progress(
        current_range,
        current_range.start,
        current_specificity,
        candidate_range,
        candidate_range.start,
        candidate_specificity,
    )
}

fn compare_failure_rank_at_progress(
    current_range: TextRange,
    current_progress: usize,
    current_specificity: usize,
    candidate_range: TextRange,
    candidate_progress: usize,
    candidate_specificity: usize,
) -> Ordering {
    let current_len = current_range.end.saturating_sub(current_range.start);
    let candidate_len = candidate_range.end.saturating_sub(candidate_range.start);

    match (current_len > 0, candidate_len > 0) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (true, true) => candidate_progress
            .cmp(&current_progress)
            .then_with(|| candidate_specificity.cmp(&current_specificity))
            .then_with(|| candidate_range.start.cmp(&current_range.start))
            // At the same start, a narrow range identifies the bad token more precisely.
            .then_with(|| current_len.cmp(&candidate_len)),
        (false, false) => candidate_specificity
            .cmp(&current_specificity)
            .then_with(|| candidate_range.start.cmp(&current_range.start)),
    }
}

pub(crate) fn compare_failure_trace_rank(
    current: &FailureTrace,
    candidate: &FailureTrace,
) -> Ordering {
    let current_root = current.root_cause();
    let candidate_root = candidate.root_cause();
    let current_range = current_root.failure.span.mapped.virtual_range;
    let candidate_range = candidate_root.failure.span.mapped.virtual_range;
    compare_failure_rank_at_progress(
        current_range,
        if current.is_event_restriction() {
            current_range.end
        } else {
            current_range.start
        },
        current.specificity(),
        candidate_range,
        if candidate.is_event_restriction() {
            candidate_range.end
        } else {
            candidate_range.start
        },
        candidate.specificity(),
    )
    // A structurally complete candidate rejected by its semantic handler is
    // more actionable than a generic structural failure at the same rank.
    .then_with(|| {
        candidate
            .is_semantic_rejection()
            .cmp(&current.is_semantic_rejection())
    })
}

/// Keeps the most actionable of two recoverable parser failures.
pub fn choose_failure_trace(
    current: Option<FailureTrace>,
    candidate: Option<FailureTrace>,
) -> Option<FailureTrace> {
    match (current, candidate) {
        (None, value) | (value, None) => value,
        (Some(current), Some(candidate)) => {
            if compare_failure_trace_rank(&current, &candidate) == Ordering::Greater {
                Some(candidate)
            } else {
                Some(current)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MappedSource, PatternFailureReason, TextRange};

    fn trace(
        source: &MappedSource,
        root_range: TextRange,
        parent_ranges: &[TextRange],
    ) -> FailureTrace {
        let span = MatchSpan {
            local_range: root_range,
            mapped: source.map_range(root_range).expect("range must map"),
        };
        parent_ranges.iter().copied().fold(
            FailureTrace::leaf(PatternFailure {
                span,
                reasons: vec![PatternFailureReason::Expression],
            }),
            |trace, input_range| {
                trace.with_parent(FailureFrame {
                    kind: MatchSyntaxKind::Expression,
                    definition_id: "definition".to_owned(),
                    registration_id: "registration".to_owned(),
                    pattern_index: 0,
                    pattern: "%objects% if <.+>[,] (otherwise|else) %objects%".to_owned(),
                    element_path: Vec::new(),
                    pattern_span: None,
                    input_span: MatchSpan {
                        local_range: input_range,
                        mapped: source.map_range(input_range).expect("range must map"),
                    },
                    role: FailureFrameRole::TypeExpressionCapture,
                })
            },
        )
    }

    #[test]
    fn deeper_semantic_provenance_beats_a_later_shallow_failure() {
        let source = MappedSource::identity("send 1 if a < 5 else 2");
        let deeper = trace(
            &source,
            TextRange::new(10, 11),
            &[TextRange::new(7, 22), TextRange::new(0, 22)],
        );
        let later = trace(&source, TextRange::empty(22), &[]);

        assert_eq!(
            choose_failure_trace(Some(later), Some(deeper.clone())),
            Some(deeper)
        );
    }

    #[test]
    fn later_root_range_wins_at_equal_specificity() {
        let source = MappedSource::identity("0123456789");
        let earlier = trace(&source, TextRange::new(2, 3), &[]);
        let later = trace(&source, TextRange::new(7, 8), &[]);

        assert_eq!(
            choose_failure_trace(Some(earlier), Some(later.clone())),
            Some(later)
        );
    }

    #[test]
    fn later_concrete_failure_beats_earlier_deeper_concrete_failure() {
        let source = MappedSource::identity("send 1 and あ");
        let earlier = trace(
            &source,
            TextRange::new(5, 6),
            &[TextRange::new(5, 6), TextRange::new(0, 14)],
        );
        let later = trace(&source, TextRange::new(11, 14), &[TextRange::new(7, 14)]);

        assert_eq!(
            choose_failure_trace(Some(earlier), Some(later.clone())),
            Some(later)
        );
    }

    #[test]
    fn equal_root_start_prefers_non_empty_then_shorter_ranges() {
        let source = MappedSource::identity("0123456789");
        let empty = trace(&source, TextRange::empty(2), &[]);
        let non_empty = trace(&source, TextRange::new(2, 6), &[]);
        assert_eq!(
            choose_failure_trace(Some(empty), Some(non_empty.clone())),
            Some(non_empty)
        );

        let broad = trace(&source, TextRange::new(2, 8), &[]);
        let narrow = trace(&source, TextRange::new(2, 4), &[]);
        assert_eq!(
            choose_failure_trace(Some(broad), Some(narrow.clone())),
            Some(narrow)
        );
    }

    #[test]
    fn containing_event_restriction_wins_over_partial_syntax_failure() {
        let source = MappedSource::identity("final damage");
        let partial = trace(&source, TextRange::new(0, 5), &[]);
        let restricted = FailureTrace::leaf(PatternFailure {
            span: MatchSpan {
                local_range: TextRange::new(0, 12),
                mapped: source.map_range(TextRange::new(0, 12)).unwrap(),
            },
            reasons: vec![PatternFailureReason::EventRestricted {
                supported: vec!["EntityDamageEvent".to_owned()],
                current: Vec::new(),
            }],
        });

        assert_eq!(
            choose_failure_trace(Some(partial.clone()), Some(restricted.clone())),
            Some(restricted.clone())
        );
        assert_eq!(
            choose_failure_trace(Some(restricted.clone()), Some(partial)),
            Some(restricted)
        );
    }

    #[test]
    fn failure_trace_ranking_is_transitive_around_event_restrictions() {
        let source = MappedSource::identity("012345678901");
        let event = FailureTrace::leaf(PatternFailure {
            span: MatchSpan {
                local_range: TextRange::new(0, 10),
                mapped: source.map_range(TextRange::new(0, 10)).unwrap(),
            },
            reasons: vec![PatternFailureReason::EventRestricted {
                supported: vec!["TestEvent".to_owned()],
                current: Vec::new(),
            }],
        });
        let inner = trace(&source, TextRange::new(5, 6), &[]);
        let overlapping = trace(&source, TextRange::new(4, 12), &[]);

        assert_eq!(compare_failure_trace_rank(&event, &inner), Ordering::Less);
        assert_eq!(
            compare_failure_trace_rank(&inner, &overlapping),
            Ordering::Less
        );
        assert_eq!(
            compare_failure_trace_rank(&event, &overlapping),
            Ordering::Less
        );
    }

    #[test]
    fn semantic_diagnostics_follow_the_ranked_failure() {
        let source = MappedSource::identity("0123456789");
        let earlier = trace(&source, TextRange::new(2, 3), &[]).with_semantic_diagnostics(vec![
            SemanticDiagnostic {
                code: "earlier".to_owned(),
                message: "earlier candidate".to_owned(),
                severity: SemanticDiagnosticSeverity::Error,
                span: source.map_range(TextRange::new(2, 3)).unwrap(),
                related: Vec::new(),
            },
        ]);
        let later = trace(&source, TextRange::new(7, 8), &[]).with_semantic_diagnostics(vec![
            SemanticDiagnostic {
                code: "later".to_owned(),
                message: "later candidate".to_owned(),
                severity: SemanticDiagnosticSeverity::Warning,
                span: source.map_range(TextRange::new(7, 8)).unwrap(),
                related: Vec::new(),
            },
        ]);

        let selected = choose_failure_trace(Some(earlier), Some(later)).unwrap();
        let codes = selected
            .semantic_diagnostics()
            .into_iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();
        assert_eq!(codes, ["later"]);
    }

    #[test]
    fn semantic_rejection_breaks_an_otherwise_equal_rank() {
        let source = MappedSource::identity("0123456789");
        let structural = trace(&source, TextRange::new(0, 10), &[]);
        let semantic = FailureTrace::leaf(PatternFailure {
            span: MatchSpan {
                local_range: TextRange::new(0, 10),
                mapped: source.map_range(TextRange::new(0, 10)).unwrap(),
            },
            reasons: vec![PatternFailureReason::HookRejected {
                reason: "semantic rejection".to_owned(),
            }],
        })
        .with_parent(FailureFrame {
            kind: MatchSyntaxKind::Expression,
            definition_id: "semantic-definition".to_owned(),
            registration_id: "semantic-registration".to_owned(),
            pattern_index: 0,
            pattern: "%objects% parsed as %classinfo%".to_owned(),
            element_path: Vec::new(),
            pattern_span: None,
            input_span: MatchSpan {
                local_range: TextRange::new(0, 10),
                mapped: source.map_range(TextRange::new(0, 10)).unwrap(),
            },
            role: FailureFrameRole::SemanticCandidate,
        });

        assert_eq!(
            choose_failure_trace(Some(structural), Some(semantic.clone())),
            Some(semantic)
        );
    }
}
