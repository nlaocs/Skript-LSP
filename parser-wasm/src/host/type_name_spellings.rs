//! Bounded editing spellings, revalidated by the authoritative ordered matcher.
use super::TypeUserInputMatcher;
use fancy_regex::Expr;
use std::collections::HashSet;

const MAX_TYPES: usize = 4096;
const MAX_VARIANTS: usize = 64;
const MAX_BYTES: usize = 256;

pub(super) fn collect(
    matchers: &[TypeUserInputMatcher],
    catalog: Option<&syntaxes::Catalog>,
) -> (Vec<(String, String)>, bool) {
    let mut names = Vec::new();
    let mut attempted = HashSet::new();
    let mut limited = matchers.len() > MAX_TYPES;
    let mut checks = 0usize;
    let mut checked_bytes = 0usize;
    let mut expanded_bytes = 0usize;
    for matcher in matchers.iter().take(MAX_TYPES) {
        let option = &matcher.option;
        let mut proposals = Vec::new();
        for text in [&option.code_name, &option.singular, &option.plural] {
            if text.len() <= MAX_BYTES && !text.chars().any(char::is_control) {
                proposals.push(text.clone());
            } else {
                limited = true;
            }
        }
        limited |= option.user_input_patterns.len() > 64;
        for source in option.user_input_patterns.iter().take(64) {
            if expanded_bytes + source.len() > 1024 * 1024 {
                return (names, true);
            }
            expanded_bytes += source.len();
            let expanded = (source.len() <= 1024)
                .then(|| Expr::parse_tree(source).ok())
                .flatten()
                .and_then(|tree| finite(&tree.expr, 0, &mut 2048));
            match expanded {
                Some(values) => proposals.extend(values),
                None => limited = true,
            }
        }
        for text in proposals {
            if text.is_empty()
                || text.len() > MAX_BYTES
                || text.chars().any(char::is_control)
                || text.split_whitespace().collect::<Vec<_>>().join(" ") != text
                || !attempted.insert(text.clone())
            {
                continue;
            }
            // Core strips one article before calling the ordered host matcher.
            // Revalidate that exact input, preserving the original insert text.
            let input = crate::type_language::strip_indefinite_article(&text, |key| {
                catalog
                    .and_then(|c| c.language_value(key))
                    .map(str::to_owned)
            });
            let mut matched = None;
            'ordered: for candidate in matchers.iter().take(MAX_TYPES) {
                for pattern in &candidate.patterns {
                    if checks == 262_144 || checked_bytes + text.len() > 32 * 1024 * 1024 {
                        return (names, true);
                    }
                    checks += 1;
                    checked_bytes += text.len();
                    match pattern.is_match(input) {
                        Ok(true) => {
                            matched = Some(&candidate.option);
                            break 'ordered;
                        }
                        Ok(false) => {}
                        Err(_) => return (names, true),
                    }
                }
            }
            if let Some(option) = matched {
                if names.len() == MAX_TYPES {
                    return (names, true);
                }
                names.push((text, option.registration_id.clone()));
            }
        }
    }
    (names, limited)
}

fn product(left: &[String], right: &[String]) -> Option<Vec<String>> {
    let mut result = Vec::new();
    for a in left {
        for b in right {
            if a.len() + b.len() > MAX_BYTES {
                return None;
            }
            let text = format!("{a}{b}");
            if !result.contains(&text) {
                if result.len() == MAX_VARIANTS {
                    return None;
                }
                result.push(text);
            }
        }
    }
    Some(result)
}

fn finite(expr: &Expr, depth: usize, budget: &mut usize) -> Option<Vec<String>> {
    if depth == 32 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    match expr {
        Expr::Empty => Some(vec![String::new()]),
        Expr::Literal { val, .. } if val.len() <= MAX_BYTES => Some(vec![val.clone()]),
        Expr::Group(child) => finite(child, depth + 1, budget),
        Expr::Concat(children) => {
            let mut result = vec![String::new()];
            for child in children {
                result = product(&result, &finite(child, depth + 1, budget)?)?;
            }
            Some(result)
        }
        Expr::Alt(children) => {
            let mut result = Vec::new();
            for child in children {
                for text in finite(child, depth + 1, budget)? {
                    if !result.contains(&text) {
                        if result.len() == MAX_VARIANTS {
                            return None;
                        }
                        result.push(text);
                    }
                }
            }
            Some(result)
        }
        Expr::Repeat { child, lo, hi, .. } if *hi <= 8 => {
            let child = finite(child, depth + 1, budget)?;
            let mut result = Vec::new();
            let mut sequence = vec![String::new()];
            for count in 0..=*hi {
                if count >= *lo {
                    for text in &sequence {
                        if !result.contains(text) {
                            if result.len() == MAX_VARIANTS {
                                return None;
                            }
                            result.push(text.clone());
                        }
                    }
                }
                if count < *hi {
                    sequence = product(&sequence, &child)?;
                }
            }
            Some(result)
        }
        // Character classes, lookaround, backreferences and unbounded repeats
        // stay opaque. Noun/code fallbacks are still checked by the real matcher.
        _ => None,
    }
}
