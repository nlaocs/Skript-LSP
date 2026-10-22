//! Finite editing proposals for declared routed regex captures. These are
//! suggestions, never successful semantic parses or output-type witnesses.
use super::*;
use skript_parser::{
    MatchInput, MatchPattern, MatchSyntaxKind, NoopPatternMatchHooks, PatternCandidate,
    PatternMatcherConfig, RegisteredCaptureBinding, RejectTypeExpressions,
};

const MAX_VALUES: usize = 256;
const MAX_RECORDS: usize = 512;
const MAX_SUPPLEMENTS: usize = 4096;
const MAX_ATTEMPTS: usize = 16_384;
const MAX_PATTERN_ATTEMPTS: usize = 1024;
const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn rebuild(
    index: &mut CatalogIndex,
    catalog: &Catalog,
    routes: &HashMap<String, Vec<RegisteredCaptureBinding>>,
) {
    index.templates.retain(|template| !template.finite_capture);
    let mut order = HashMap::new();
    for syntax in catalog.syntaxes() {
        if let Some(common) = syntax.common() {
            for pattern in 0..common.patterns.len() {
                order.insert(
                    format!("{}:{pattern}", common.registration_id.as_str()),
                    order.len(),
                );
            }
        }
    }
    let mut seen = index
        .templates
        .iter()
        .map(|t| (t.identity.clone(), render(&t.parts, false)))
        .collect::<HashSet<_>>();
    let mut values = HashMap::<Vec<String>, (Vec<String>, bool)>::new();
    let mut attempts = 0;
    let mut input_bytes = 0;
    let mut supplements = 0;
    for syntax in catalog.syntaxes() {
        let Syntax::Expression(expression) = syntax else {
            continue;
        };
        let common = &expression.common;
        for (pattern_index, pattern) in common.patterns.iter().enumerate() {
            let identity = format!("{}:{pattern_index}", common.registration_id.as_str());
            let Some(route) = routes.get(&identity).and_then(|routes| {
                // v1 supports one routed regex and no typed/nested placeholders.
                (routes.len() == 1).then(|| &routes[0])
            }) else {
                continue;
            };
            if route.capture_index != 0
                || route.parser_id != "host.expression"
                || !skript_parser::capture_expression_flags(route).is_ok_and(|(literal, _)| literal)
            {
                continue;
            }
            let Some(base) = index.patterns.iter().find(|p| p.identity == identity) else {
                continue;
            };
            if base.graph.parameters.len() != 1 || base.graph.slot_types[0].is_some() {
                continue;
            }
            let Ok(expected) = skript_parser::capture_expression_expected_types(route) else {
                continue;
            };
            if expected.is_empty() {
                continue;
            }
            let classes = expected
                .iter()
                .map(|e| e.class_name.0.clone())
                .collect::<Vec<_>>();
            let options = values
                .entry(classes.clone())
                .or_insert_with(|| finite_values(catalog, &classes));
            index.limited |= options.1;
            let mut added = Vec::new();
            let mut pattern_attempts = 0;
            for value in &options.0 {
                let Some(elements) = replace_regex(&pattern.parsed.elements, value, 0) else {
                    continue;
                };
                for parts in variants(&elements, 0, &mut index.limited, catalog.plural_rules()) {
                    let parts = normalize_parts(parts);
                    if parts.iter().any(|p| matches!(p, Part::Slot(_))) {
                        continue;
                    }
                    let text = render(&parts, false);
                    if text.is_empty() || text.len() > MAX_TEMPLATE_BYTES {
                        continue;
                    }
                    if seen.contains(&(identity.clone(), text.clone())) {
                        continue;
                    }
                    // Charge failed matches as well as successful proposals.
                    if pattern_attempts == MAX_PATTERN_ATTEMPTS {
                        index.limited = true;
                        break;
                    }
                    if attempts == MAX_ATTEMPTS
                        || input_bytes + text.len() > MAX_INPUT_BYTES
                        || supplements == MAX_SUPPLEMENTS
                    {
                        index.limited = true;
                        insert(index, added, &order, &identity);
                        return;
                    }
                    pattern_attempts += 1;
                    attempts += 1;
                    input_bytes += text.len();
                    match lexically_matches(pattern, pattern_index, &text, &identity) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(()) => {
                            index.limited = true;
                            continue;
                        }
                    }
                    seen.insert((identity.clone(), text.clone()));
                    supplements += 1;
                    added.push(Template {
                        finite_capture: true,
                        documentation: base.documentation.clone(),
                        kind: base.kind,
                        label: base.label.clone(),
                        detail: format!(
                            "{}; finite capture value (context unverified)",
                            base.detail
                        ),
                        identity: identity.clone(),
                        folded: text.to_lowercase(),
                        leading: text,
                        parts,
                    });
                }
                if pattern_attempts == MAX_PATTERN_ATTEMPTS {
                    index.limited = true;
                    break;
                }
            }
            insert(index, added, &order, &identity);
        }
    }
}

fn finite_values(catalog: &Catalog, classes: &[String]) -> (Vec<String>, bool) {
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    let mut records = 0;
    for syntax in catalog.syntaxes() {
        let Syntax::Type(ty) = syntax else { continue };
        // Exact routed Type only: no converter or represented-class inference.
        if !ty.has_parser
            || !classes
                .iter()
                .any(|class| class == ty.original_class.as_str())
        {
            continue;
        }
        for literal in &ty.type_literals {
            if records == MAX_RECORDS || values.len() == MAX_VALUES {
                return (values, true);
            }
            records += 1;
            let text = &literal.text;
            if !supported_literal(text) || !seen.insert(text.clone()) {
                continue;
            }
            values.push(text.clone());
        }
    }
    (values, false)
}
fn supported_literal(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 256
        && !text.chars().any(char::is_control)
        && text.split_whitespace().collect::<Vec<_>>().join(" ") == text
}
fn replace_regex(
    elements: &[SpannedPatternElement],
    value: &str,
    depth: usize,
) -> Option<Vec<SpannedPatternElement>> {
    if depth > 32 {
        return None;
    }
    let mut result = elements.to_vec();
    for element in &mut result {
        match &mut element.value {
            PatternElement::Regex(_) => element.value = PatternElement::Literal(value.into()),
            PatternElement::TypeExpr(_) => return None,
            PatternElement::Group(children) | PatternElement::Option(children) => {
                *children = replace_regex(children, value, depth + 1)?
            }
            PatternElement::Choice(branches) => {
                for branch in branches {
                    *branch = replace_regex(branch, value, depth + 1)?;
                }
            }
            _ => {}
        }
    }
    Some(result)
}
fn lexically_matches(
    pattern: &syntaxes::Pattern,
    pattern_index: usize,
    text: &str,
    identity: &str,
) -> Result<bool, ()> {
    let source = skript_parser::MappedSource::identity(text);
    let Ok(input) = MatchInput::from_source(&source, TextRange::new(0, text.len())) else {
        return Err(());
    };
    let candidate = PatternCandidate {
        kind: MatchSyntaxKind::Expression,
        definition_id: identity.into(),
        registration_id: identity.into(),
        priority: 0,
        registration_order: 0,
        resolved_order: None,
        patterns: vec![MatchPattern {
            pattern_index,
            source: &pattern.source,
            parsed: &pattern.parsed,
        }],
    };
    skript_parser::match_pattern_candidates(
        input,
        &[candidate],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig {
            max_states: 256,
            max_backtracks: 128,
            max_regex_executions: 32,
            max_regex_evaluated_bytes: 16 * 1024,
            max_regex_backtracks: 1024,
            max_candidate_failures: 1,
            recover_type_expression_failures: false,
        },
    )
    .map(|result| result.selected.is_some())
    .map_err(|_| ())
}
fn insert(
    index: &mut CatalogIndex,
    added: Vec<Template>,
    order: &HashMap<String, usize>,
    identity: &str,
) {
    // Insert by Catalog pattern order without moving any existing proposal.
    let rank = order[identity];
    let at = index
        .templates
        .iter()
        .position(|t| order.get(&t.identity).is_some_and(|other| *other > rank))
        .unwrap_or(index.templates.len());
    index.templates.splice(at..at, added);
}

#[cfg(test)]
mod tests;
