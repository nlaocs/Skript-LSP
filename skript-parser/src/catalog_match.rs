use crate::{MatchPattern, MatchSyntaxKind, PatternCandidate};
use syntaxes::{
    Catalog, DynamicSyntaxSnapshot, Pattern, Syntax, SyntaxCandidateSource, SyntaxKind,
};

pub fn catalog_pattern_candidates(
    catalog: &Catalog,
    kind: SyntaxKind,
) -> Vec<PatternCandidate<'_>> {
    catalog
        .syntaxes()
        .iter()
        .filter(|syntax| syntax.kind() == kind)
        .filter_map(|syntax| static_candidate(syntax, None))
        .collect()
}

/// Builds candidates in the order already resolved by the dynamic registry.
///
/// Supplying resolved_order prevents the matcher from discarding before/after
/// constraints while the original priority and registration order remain
/// available in diagnostics and match results.
pub fn snapshot_pattern_candidates<'a>(
    catalog: &'a Catalog,
    snapshot: &'a DynamicSyntaxSnapshot,
    kind: SyntaxKind,
) -> Vec<PatternCandidate<'a>> {
    snapshot
        .candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.kind == kind)
        .filter_map(|(resolved_order, candidate)| match &candidate.source {
            SyntaxCandidateSource::Static(index) => catalog
                .syntax_at(*index)
                .and_then(|syntax| static_candidate(syntax, Some(resolved_order))),
            SyntaxCandidateSource::Dynamic(id) => {
                let definition = snapshot.definitions.get(id)?;
                Some(PatternCandidate {
                    kind: match_kind(definition.kind),
                    definition_id: definition.id.qualified(),
                    registration_id: definition.id.qualified(),
                    priority: definition.priority,
                    registration_order: usize::try_from(definition.declaration_order)
                        .unwrap_or(usize::MAX),
                    resolved_order: Some(resolved_order),
                    patterns: definition
                        .patterns
                        .iter()
                        .map(|pattern| MatchPattern {
                            source: &pattern.source,
                            parsed: &pattern.parsed,
                        })
                        .collect(),
                })
            }
        })
        .collect()
}

fn static_candidate(
    syntax: &Syntax,
    resolved_order: Option<usize>,
) -> Option<PatternCandidate<'_>> {
    let patterns = syntax_patterns(syntax)?;
    Some(PatternCandidate {
        kind: match_kind(syntax.kind()),
        definition_id: syntax.definition_id().as_str().to_owned(),
        registration_id: syntax.registration_id().as_str().to_owned(),
        priority: 0,
        registration_order: syntax.registration_order(),
        resolved_order,
        patterns: patterns
            .iter()
            .map(|pattern| MatchPattern {
                source: &pattern.source,
                parsed: &pattern.parsed,
            })
            .collect(),
    })
}

fn syntax_patterns(syntax: &Syntax) -> Option<&[Pattern]> {
    match syntax {
        Syntax::Event(value) => Some(&value.common.patterns),
        Syntax::Condition(value) => Some(&value.common.patterns),
        Syntax::Effect(value) => Some(&value.common.patterns),
        Syntax::Expression(value) => Some(&value.common.patterns),
        Syntax::Type(_) | Syntax::Function(_) => None,
        Syntax::Section(value) => Some(&value.common.patterns),
        Syntax::Structure(value) => Some(&value.common.patterns),
    }
}

pub const fn match_kind(kind: SyntaxKind) -> MatchSyntaxKind {
    match kind {
        SyntaxKind::Event => MatchSyntaxKind::Event,
        SyntaxKind::Condition => MatchSyntaxKind::Condition,
        SyntaxKind::Effect => MatchSyntaxKind::Effect,
        SyntaxKind::Expression => MatchSyntaxKind::Expression,
        SyntaxKind::Type => MatchSyntaxKind::Type,
        SyntaxKind::Function => MatchSyntaxKind::Function,
        SyntaxKind::Section => MatchSyntaxKind::Section,
        SyntaxKind::Structure => MatchSyntaxKind::Structure,
    }
}
