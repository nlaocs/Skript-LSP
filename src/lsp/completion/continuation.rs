//! Structural pattern frontiers. Slots are opaque balanced source, not type proofs.
use super::word;
use std::collections::{BTreeSet, HashMap, HashSet};
use syntax_pattern_parser::syntax::{PatternElement, SpannedPatternElement};

const MAX_NODES: usize = 2048;
const MAX_STATES: usize = 4096;
const MAX_SLOT_ENDS: usize = 128;

/// Shared by every pattern in one request, including source character scans.
pub(super) struct Budget(usize);
impl Budget {
    pub fn new() -> Self {
        Self(1_000_000)
    }
    fn spend(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
    pub fn exhausted(&self) -> bool {
        self.0 == 0
    }
    fn scan(&mut self, bytes: usize) -> bool {
        if bytes > self.0 {
            self.0 = 0;
            return false;
        }
        self.0 -= bytes;
        true
    }
}

#[derive(Clone)]
enum Edge {
    Empty(usize),
    Text(String, usize),
    Slot(usize),
}

pub(super) struct Graph {
    nodes: Vec<Vec<Edge>>,
    end: usize,
}

pub(super) struct Frontier {
    pub words: Vec<(String, usize, usize)>,
    pub limited: bool,
}

impl Graph {
    #[cfg(test)]
    pub fn new(elements: &[SpannedPatternElement]) -> Option<Self> {
        Self::with_prefix("", elements)
    }
    pub fn with_prefix(prefix: &str, elements: &[SpannedPatternElement]) -> Option<Self> {
        let mut graph = Self {
            nodes: vec![Vec::new()],
            end: 0,
        };
        let start = if prefix.is_empty() {
            0
        } else {
            let next = graph.node()?;
            graph.nodes[0].push(Edge::Text(prefix.into(), next));
            next
        };
        graph.end = graph.sequence(elements, start, 0)?;
        Some(graph)
    }

    fn node(&mut self) -> Option<usize> {
        if self.nodes.len() >= MAX_NODES {
            return None;
        }
        self.nodes.push(Vec::new());
        Some(self.nodes.len() - 1)
    }

    fn sequence(
        &mut self,
        elements: &[SpannedPatternElement],
        mut from: usize,
        depth: usize,
    ) -> Option<usize> {
        if depth > 32 {
            return None;
        }
        for element in elements {
            let next = self.node()?;
            match &element.value {
                PatternElement::Literal(text) => {
                    self.nodes[from].push(Edge::Text(text.clone(), next))
                }
                PatternElement::TypeExpr(_) | PatternElement::Regex(_) => {
                    self.nodes[from].push(Edge::Slot(next))
                }
                PatternElement::Option(elements) => {
                    self.nodes[from].push(Edge::Empty(next));
                    let end = self.sequence(elements, from, depth + 1)?;
                    self.nodes[end].push(Edge::Empty(next));
                }
                PatternElement::Choice(branches) => {
                    for branch in branches {
                        let end = self.sequence(branch, from, depth + 1)?;
                        self.nodes[end].push(Edge::Empty(next));
                    }
                }
                PatternElement::Group(elements) => {
                    let end = self.sequence(elements, from, depth + 1)?;
                    self.nodes[end].push(Edge::Empty(next));
                }
                PatternElement::ParseTag(_)
                | PatternElement::ParseMark(_)
                | PatternElement::Empty => self.nodes[from].push(Edge::Empty(next)),
            }
            from = next;
        }
        Some(from)
    }

    pub fn next_words(&self, committed: &str, query: &str, budget: &mut Budget) -> Frontier {
        let mut pending = vec![(0, 0, 0, 0)];
        let mut visited = HashSet::new();
        let mut words = HashMap::<String, (usize, usize)>::new();
        let mut limited = false;
        while let Some((node, offset, fixed, slots)) = pending.pop() {
            if !budget.spend() {
                limited = true;
                break;
            }
            if !visited.insert((node, offset, fixed, slots)) {
                continue;
            }
            if visited.len() > MAX_STATES {
                limited = true;
                break;
            }
            if node == self.end {
                continue;
            }
            // Visit branches predictably; equal-score output uses spelling order.
            for edge in self.nodes[node].iter().rev() {
                if !budget.spend() {
                    limited = true;
                    break;
                }
                match edge {
                    Edge::Empty(next) => pending.push((*next, offset, fixed, slots)),
                    Edge::Text(text, next) => match consume(text, committed, offset, budget) {
                        Some(Consumed::Full(end, matched)) => {
                            pending.push((*next, end, fixed + matched, slots))
                        }
                        Some(Consumed::Prefix(remaining, matched)) if fixed + matched > 0 => {
                            let (keywords, clipped) = self.word_frontiers(remaining, *next, budget);
                            limited |= clipped;
                            for keyword in keywords {
                                if !budget.scan(keyword.len().saturating_mul(3)) {
                                    limited = true;
                                    break;
                                }
                                if !keyword.to_lowercase().starts_with(query) {
                                    continue;
                                }
                                let score = (fixed + matched, slots);
                                words
                                    .entry(keyword)
                                    .and_modify(|old| {
                                        if score.0 > old.0 || score.0 == old.0 && score.1 < old.1 {
                                            *old = score;
                                        }
                                    })
                                    .or_insert(score);
                            }
                        }
                        _ => {}
                    },
                    Edge::Slot(next) if fixed > 0 && offset < committed.len() => {
                        let (ends, clipped) = slot_ends(committed, offset, budget);
                        limited |= clipped;
                        pending.extend(ends.into_iter().map(|end| (*next, end, fixed, slots + 1)));
                    }
                    Edge::Slot(_) => {}
                }
            }
        }
        let mut words = words
            .into_iter()
            .map(|(word, (fixed, slots))| (word, fixed, slots))
            .collect::<Vec<_>>();
        // Equal scores retain a stable spelling order, rather than HashMap order.
        words.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)).then(a.0.cmp(&b.0)));
        Frontier {
            words,
            limited: limited || budget.exhausted(),
        }
    }

    /// A literal edge may end in the middle of a word, e.g. `us[ag]e`.
    /// Follow only fixed fragments until a word boundary, without expanding
    /// the complete pattern or inserting any opaque argument.
    fn word_frontiers<'a>(
        &'a self,
        text: &'a str,
        next: usize,
        budget: &mut Budget,
    ) -> (Vec<String>, bool) {
        let mut pending = vec![(text, next, String::new())];
        let mut seen = HashSet::new();
        let mut output = BTreeSet::new();
        let mut limited = false;
        while let Some((text, next, mut prefix)) = pending.pop() {
            if !budget.scan(text.len() + prefix.len() + 1) || seen.len() >= MAX_STATES {
                limited = true;
                break;
            }
            let boundary = text.char_indices().find(|(_, c)| !word(*c)).map(|(i, _)| i);
            prefix.push_str(&text[..boundary.unwrap_or(text.len())]);
            if boundary.is_some() || next == self.end {
                if !prefix.is_empty() {
                    output.insert(prefix);
                }
                continue;
            }
            if !seen.insert((next, prefix.clone())) {
                continue;
            }
            for edge in &self.nodes[next] {
                if !budget.scan(prefix.len() + 1) {
                    limited = true;
                    break;
                }
                match edge {
                    Edge::Empty(next) => pending.push(("", *next, prefix.clone())),
                    Edge::Text(text, next) => pending.push((text, *next, prefix.clone())),
                    Edge::Slot(_) => {
                        if !prefix.is_empty() {
                            output.insert(prefix.clone());
                        }
                    }
                }
            }
        }
        (output.into_iter().collect(), limited || budget.exhausted())
    }
}

enum Consumed<'a> {
    Full(usize, usize),
    Prefix(&'a str, usize),
}

fn consume<'a>(
    pattern: &'a str,
    input: &str,
    mut offset: usize,
    budget: &mut Budget,
) -> Option<Consumed<'a>> {
    let mut cursor = 0;
    let mut fixed = 0;
    while cursor < pattern.len() {
        if !budget.spend() {
            return None;
        }
        let expected = pattern[cursor..].chars().next()?;
        if expected.is_whitespace() {
            while cursor < pattern.len() && pattern[cursor..].chars().next()?.is_whitespace() {
                if !budget.spend() {
                    return None;
                }
                cursor += pattern[cursor..].chars().next()?.len_utf8();
            }
            let before = offset;
            while offset < input.len() && input[offset..].chars().next()?.is_whitespace() {
                if !budget.spend() {
                    return None;
                }
                offset += input[offset..].chars().next()?.len_utf8();
            }
            // Optional/group edges can leave adjacent or initial whitespace.
            if before == offset && offset > 0 && !input[..offset].ends_with(char::is_whitespace) {
                return None;
            }
            continue;
        }
        if offset == input.len() {
            return Some(Consumed::Prefix(&pattern[cursor..], fixed));
        }
        let actual = input[offset..].chars().next()?;
        if !expected.to_lowercase().eq(actual.to_lowercase()) {
            return None;
        }
        cursor += expected.len_utf8();
        offset += actual.len_utf8();
        fixed += 1;
    }
    Some(Consumed::Full(offset, fixed))
}

fn slot_ends(input: &str, start: usize, budget: &mut Budget) -> (Vec<usize>, bool) {
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut cursor = start;
    let mut ends = Vec::new();
    let mut limited = false;
    let mut nonempty = false;
    while cursor < input.len() {
        if !budget.spend() {
            limited = true;
            break;
        }
        let c = input[cursor..].chars().next().unwrap();
        if !quoted
            && stack.is_empty()
            && (c.is_whitespace() || matches!(c, ',' | ')' | ']' | '}' | ':'))
            && nonempty
        {
            if ends.len() == MAX_SLOT_ENDS {
                limited = true;
                break;
            }
            ends.push(cursor);
        }
        nonempty |= !c.is_whitespace();
        if c == '"' {
            if quoted && input.as_bytes().get(cursor + 1) == Some(&b'"') {
                cursor += 1;
            } else {
                quoted = !quoted;
            }
        } else if !quoted {
            match c {
                '(' => stack.push(')'),
                '[' => stack.push(']'),
                '{' => stack.push('}'),
                ')' | ']' | '}' => {
                    if stack.pop() != Some(c) {
                        return (ends, limited);
                    }
                }
                _ => {}
            }
        }
        cursor += c.len_utf8();
    }
    if !quoted && !limited && stack.is_empty() && nonempty && ends.len() < MAX_SLOT_ENDS {
        ends.push(input.len());
    }
    (ends, limited)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn keywords(pattern: &str, committed: &str, query: &str) -> Vec<String> {
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(pattern, &rules).unwrap();
        Graph::new(&parsed.elements)
            .unwrap()
            .next_words(committed, &query.to_lowercase(), &mut Budget::new())
            .words
            .into_iter()
            .map(|v| v.0)
            .collect()
    }
    #[test]
    fn arguments_are_structurally_delimited_and_never_reinserted() {
        for source in [
            "teleport player ",
            "teleport name of player ",
            "teleport location(1, 2, 3) ",
            "teleport {_p%sin(1)%} ",
        ] {
            assert_eq!(
                keywords("teleport %entities% (to|towards) %location%", source, "to"),
                ["to", "towards"]
            );
        }
        assert!(keywords("teleport %entities% to %location%", "teleport ", "t").is_empty());
        assert!(
            keywords(
                "teleport %entities% to %location%",
                "teleport location(1, ",
                "t"
            )
            .is_empty()
        );
        assert!(
            keywords(
                "teleport %entities% to %location%",
                "teleport \"unfinished ",
                "t"
            )
            .is_empty()
        );
    }
    #[test]
    fn optional_choices_and_long_choice_lists_do_not_expand_into_a_cartesian_product() {
        assert_eq!(
            keywords("cancel us[ag]e of %objects%", "cancel ", "us"),
            ["usage", "use"]
        );
        assert_eq!(
            keywords("cancel us[ag]e of %objects%", "cancel ", "usa"),
            ["usage"]
        );
        assert_eq!(
            keywords(
                "[force] teleport %entities% (to|in [the] direction of) %location%",
                "teleport player ",
                "t"
            ),
            ["to"]
        );
        assert_eq!(
            keywords(
                "[force] teleport %entities% (to|in [the] direction of) %location%",
                "force teleport player in ",
                "d"
            ),
            ["direction"]
        );
        let pattern = format!(
            "move %objects% ({})",
            (0..40)
                .map(|n| format!("option{n}"))
                .collect::<Vec<_>>()
                .join("|")
        );
        assert_eq!(keywords(&pattern, "move player ", "option39"), ["option39"]);
    }

    #[test]
    fn request_budget_is_shared_and_exhaustion_is_reported() {
        let elements = [syntax_pattern_parser::syntax::Spanned::new(
            PatternElement::Literal("move to".into()),
            syntax_pattern_parser::syntax::Span::new(0, 7),
        )];
        let graph = Graph::new(&elements).unwrap();
        let mut budget = Budget(3);
        assert!(graph.next_words("move ", "t", &mut budget).limited);
        assert!(budget.exhausted());
        let next = graph.next_words("move ", "t", &mut budget);
        assert!(next.limited && next.words.is_empty());
    }

    #[test]
    fn budget_clipping_preserves_the_same_candidate_set() {
        let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
            "../../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
        ))
        .unwrap();
        let parsed =
            syntax_pattern_parser::syntax::parse("cancel us[ag]e of %objects%", &rules).unwrap();
        let graph = Graph::new(&parsed.elements).unwrap();
        let mut partial = false;
        for units in 1..256 {
            let first = graph.next_words("cancel ", "us", &mut Budget(units));
            let second = graph.next_words("cancel ", "us", &mut Budget(units));
            assert_eq!(first.words, second.words);
            assert_eq!(first.limited, second.limited);
            partial |= first.limited && first.words.len() == 1;
        }
        assert!(
            partial,
            "exercise a budget that retains only one optional-word variant"
        );
    }
}
