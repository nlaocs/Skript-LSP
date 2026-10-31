//! Type candidates and their support are independent of value presence. The
//! enclosing semantic Document owns source/revision, node and reaching-write
//! provenance; these bounded reason kinds describe how a type fact was derived.
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::lsp) enum Certainty {
    Inferred,
    Heuristic,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::lsp) enum Evidence {
    AcceptedResult,
    ParameterBinding,
    Declaration,
    Assignment,
    StorageResult,
    /// Complete public bounds computed by an API-contract storage transfer;
    /// this is not a native-origin or effect/identity certificate.
    DeclaredTransfer,
    ControlFlowJoin,
    TypeGuard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::lsp) struct Support {
    pub certainty: Certainty,
    /// Whether every possible type is represented, not a confidence score.
    pub candidates_complete: bool,
    pub evidence: BTreeSet<Evidence>,
}

impl Support {
    pub fn inferred() -> Self {
        Self::from(Evidence::AcceptedResult)
    }

    pub fn from(evidence: Evidence) -> Self {
        Self {
            certainty: Certainty::Inferred,
            candidates_complete: true,
            evidence: BTreeSet::from([evidence]),
        }
    }

    pub fn is_proof(&self) -> bool {
        self.certainty == Certainty::Inferred && self.candidates_complete
    }

    pub fn join(&self, other: &Self) -> Self {
        Self {
            certainty: match (self.certainty, other.certainty) {
                (Certainty::Unknown, _) | (_, Certainty::Unknown) => Certainty::Unknown,
                (Certainty::Heuristic, _) | (_, Certainty::Heuristic) => Certainty::Heuristic,
                _ => Certainty::Inferred,
            },
            candidates_complete: self.candidates_complete && other.candidates_complete,
            evidence: self.evidence.union(&other.evidence).copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_completeness_and_confidence_are_orthogonal() {
        let mut partial = Support::inferred();
        partial.candidates_complete = false;
        let heuristic = Support {
            certainty: Certainty::Heuristic,
            ..Support::inferred()
        };
        let unknown = Support {
            certainty: Certainty::Unknown,
            candidates_complete: false,
            evidence: BTreeSet::new(),
        };
        assert!(Support::inferred().is_proof());
        for support in [&partial, &heuristic, &unknown] {
            assert!(!support.is_proof());
        }
        assert_eq!(partial.certainty, Certainty::Inferred);
        assert!(heuristic.candidates_complete);
        assert!(!partial.join(&heuristic).is_proof());
        assert_eq!(partial.join(&heuristic).certainty, Certainty::Heuristic);
        assert_eq!(
            Support::inferred().join(&unknown).certainty,
            Certainty::Unknown
        );
    }
}
