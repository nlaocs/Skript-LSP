//! Structured provenance for failures produced by nested syntax parsing.

use crate::{MatchSpan, MatchSyntaxKind, PatternFailure, PatternPathSegment};
use syntax_pattern_parser::syntax::Span as PatternSpan;

/// Semantic role of one syntax frame in a nested failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureFrameRole {
    /// A typed pattern capture failed during recursive Expression parsing.
    TypeExpressionCapture,
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

/// One failure with its parent syntax and optional more specific cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureTrace {
    pub failure: PatternFailure,
    pub frame: Option<FailureFrame>,
    pub cause: Option<Box<FailureTrace>>,
}

impl FailureTrace {
    /// Creates a leaf trace without surrounding syntax context.
    pub fn leaf(failure: PatternFailure) -> Self {
        Self {
            failure,
            frame: None,
            cause: None,
        }
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
}

pub(crate) fn choose_failure_trace(
    current: Option<FailureTrace>,
    candidate: Option<FailureTrace>,
) -> Option<FailureTrace> {
    match (current, candidate) {
        (None, value) | (value, None) => value,
        (Some(current), Some(candidate)) => {
            let current_root = current.root_cause();
            let candidate_root = candidate.root_cause();
            let current_range = current_root.failure.span.mapped.virtual_range;
            let candidate_range = candidate_root.failure.span.mapped.virtual_range;
            let current_len = current_range.end.saturating_sub(current_range.start);
            let candidate_len = candidate_range.end.saturating_sub(candidate_range.start);
            let candidate_is_stronger_restriction = candidate.is_event_restriction()
                && !current.is_event_restriction()
                && candidate_range.start <= current_range.start
                && candidate_range.end >= current_range.end;
            let current_is_stronger_restriction = current.is_event_restriction()
                && !candidate.is_event_restriction()
                && current_range.start <= candidate_range.start
                && current_range.end >= candidate_range.end;
            if current_is_stronger_restriction {
                return Some(current);
            }
            if candidate_is_stronger_restriction
                || (candidate.specificity() > current.specificity()
                    || (candidate.specificity() == current.specificity()
                        && (candidate_range.start > current_range.start
                            || (candidate_range.start == current_range.start
                                && candidate_len > 0
                                && (current_len == 0 || candidate_len < current_len)))))
            {
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
}
