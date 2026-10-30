//! Bounded, revision-local history. It never participates in current proof.
use super::{Fact, Facts, InferredType, VariableKey};
use crate::lsp::positions::LineIndex;
use skript_parser::TextRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Invalidation {
    Source,
    Read,
    Provider,
    Formatter,
    ControlFlow,
    Write,
    Alias,
    Shared,
}
impl Invalidation {
    fn description(self) -> &'static str {
        match self {
            Self::Source => "unavailable or unverified source/statement",
            Self::Read => "variable lookup/conversion preservation is unverified",
            Self::Provider => "provider current-frame effects are unverified",
            Self::Formatter => "intermediate formatting preservation is unverified",
            Self::ControlFlow => "loop or provider control flow is unverified",
            Self::Write => "write result/storage preservation is unverified",
            Self::Alias => "a write may overlap this variable binding",
            Self::Shared => "shared execution interval or interference assumptions expired",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TypeHistory {
    pub ty: InferredType,
    cause: Option<TextRange>,
    reason: Invalidation,
}
impl TypeHistory {
    pub fn from_current(
        fact: &Fact,
        cause: Option<TextRange>,
        reason: Invalidation,
    ) -> Option<Self> {
        (!matches!(fact.ty, InferredType::Unknown)).then(|| Self {
            ty: fact.ty.clone(),
            cause,
            reason,
        })
    }
    pub fn join(left: Option<&Self>, right: Option<&Self>) -> Option<Self> {
        // Divergent histories have no single common cause. Never manufacture a
        // textual last-use transition or merge historical types into current.
        left.filter(|left| right == Some(*left)).cloned()
    }
    pub fn description(&self, source: &str, lines: &LineIndex) -> String {
        let at = self
            .cause
            .and_then(|range| lines.range(source, range).ok())
            .map(|range| {
                format!(
                    " (line {}, column {})",
                    range.start.line + 1,
                    range.start.character + 1
                )
            })
            .unwrap_or_default();
        format!(
            "{}{}; previous type is not a current guarantee",
            self.reason.description(),
            at
        )
    }
}
/// Isolate an opaque operand without losing explanatory prior knowledge.
/// Every copied entry is history-only: no current type or reaching proof.
pub(super) fn historical_facts(facts: &Facts) -> Facts {
    facts
        .iter()
        .filter_map(|(key, fact)| {
            let history = fact.history.clone();
            if history.is_none()&&fact.candidate.is_none(){return None;}
            Some((
                key.clone(),
                Fact {
                    ty: InferredType::Unknown,
                    definitions: Vec::new(),
                    alias: fact.alias.clone(),
                    list: fact.list,
                    history,
                    candidate:if key.local {None}else{fact.candidate.clone()},
                },
            ))
        })
        .take(4096)
        .collect()
}
pub(super) fn invalidate(facts: &mut Facts, cause: Option<TextRange>, reason: Invalidation) {
    invalidate_where(facts, cause, reason, |_, _| true);
}
pub(super) fn invalidate_where(
    facts: &mut Facts,
    cause: Option<TextRange>,
    reason: Invalidation,
    affected: impl Fn(&VariableKey, &Fact) -> bool,
) {
    for (key, fact) in facts.iter_mut() {
        if affected(key, fact) {
            if !key.local {
                if matches!(reason,Invalidation::Alias|Invalidation::Write|Invalidation::Source){fact.candidate=None;}
                else if let Some(candidate)=super::shared::Candidate::new(&fact.ty){fact.candidate=Some(candidate);}
            }
            if let Some(history) = TypeHistory::from_current(fact, cause, reason) {
                fact.history = Some(history);
            }
            fact.ty = InferredType::Unknown;
            // Preserve previous navigation behavior: an unverified write cannot
            // revive old reaching definitions or make rename safe again.
            fact.definitions.clear();
        }
    }
    let mut histories = 0;
    facts.retain(|_, fact| {
        if fact.history.is_some() {
            histories += 1;
            if histories > 4096 {
                fact.history = None;
            }
        }
        !matches!(fact.ty, InferredType::Unknown)
            || !fact.definitions.is_empty()
            || fact.history.is_some()
            || fact.candidate.is_some()
    });
}
