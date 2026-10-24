//! Positive grammar guidance through provider-declared condition captures.
//! These routes never certify argument acceptance or participate in exclusions.
use super::*;

impl Document {
    pub(super) fn continuation_words<'a>(
        &'a self,
        line: &Line,
        committed: &str,
        query: &str,
        budget: &mut continuation::Budget,
    ) -> (Vec<(usize, usize, usize, String, &'a Pattern)>, bool) {
        let routes_current = self.index.typing.as_ref()
            .is_some_and(typing::Index::finite_captures_current);
        let mut pending = vec![(0, false, 0, 0, 0)];
        let mut visited = HashMap::<(usize, bool), Vec<(usize, usize, usize)>>::new();
        let mut output = Vec::new();
        let mut limited = false;
        while let Some((start, condition, outer_fixed, outer_slots, depth)) = pending.pop() {
            let states = visited.entry((start, condition)).or_default();
            if !budget.scan(states.len() + 1) {
                limited = true;
                break;
            }
            // Revisit a converging capture when a shallower path can reach
            // further, or when its fixed/slot evidence can rank words better.
            if states.iter().any(|&(d, f, s)|
                d <= depth && f >= outer_fixed && s <= outer_slots) {
                continue;
            }
            states.retain(|&(d, f, s)|
                !(depth <= d && outer_fixed >= f && outer_slots <= s));
            states.push((depth, outer_fixed, outer_slots));
            let text = &committed[start..];
            for (order, pattern) in self.index.patterns.iter().enumerate() {
                if !budget.scan(1) {
                    limited = true;
                    break;
                }
                let allowed = if condition || matches!(line.scope, Scope::Conditions) {
                    pattern.kind == SyntaxKind::Condition
                } else if line.root {
                    matches!(pattern.kind, SyntaxKind::Event | SyntaxKind::Structure)
                } else {
                    matches!(pattern.kind, SyntaxKind::Effect | SyntaxKind::Condition | SyntaxKind::Section)
                };
                if !allowed {
                    continue;
                }
                let frontier = pattern.graph.next_words(text, query, budget);
                limited |= frontier.limited;
                output.extend(frontier.words.into_iter().map(|(word, fixed, slots)|
                    (outer_fixed + fixed, outer_slots + slots, order, word, pattern)));
                if !routes_current {
                    continue;
                }
                let Some(routes) = self.index.capture_routes.get(&pattern.identity) else {
                    continue;
                };
                for (parameter, offset, fixed, slots) in frontier.captures {
                    if !routes.iter().any(|(index, parser)| *index == parameter
                        && parser == "host.condition") {
                        continue;
                    }
                    if depth == 12 {
                        limited = true;
                        continue;
                    }
                    pending.push((start + offset, true, outer_fixed + fixed,
                        outer_slots + slots, depth + 1));
                }
            }
            if budget.exhausted() {
                break;
            }
        }
        (output, limited || budget.exhausted())
    }
}
