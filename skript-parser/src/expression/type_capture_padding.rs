//! Raw matcher consumption and exact expression spans have different roles.
use super::{HashMap, TextRange, java_trim_range};

pub(super) struct Prefixes {
    pub range: TextRange,
    pub ends: Vec<usize>,
    raw: Option<HashMap<usize, Vec<usize>>>,
}

impl Prefixes {
    /// The caller validates the raw source range and offered UTF-8 endpoints.
    pub fn new(input: &str, range: TextRange, ends: Vec<usize>) -> Self {
        let start = range.start + java_trim_range(range.slice(input).unwrap()).start;
        if start == range.start && ends.iter().all(|end|
            input.as_bytes().get(end.saturating_sub(1)).is_none_or(|byte| *byte > b' ')) {
            return Self { range, ends, raw: None };
        }
        // Scan each disjoint source segment once; a long whitespace run must
        // not be rescanned separately for every offered raw endpoint.
        let mut ordered = ends.clone();
        if !ordered.is_sorted() { ordered.sort_unstable(); }
        ordered.dedup();
        let mut cursor = start;
        let mut semantic_end = start;
        let mut trimmed = HashMap::new();
        for end in ordered.into_iter().filter(|end| *end > start) {
            let part = java_trim_range(&input[cursor..end]);
            if !part.is_empty() { semantic_end = cursor + part.end; }
            if semantic_end > start { trimmed.insert(end, semantic_end); }
            cursor = end;
        }
        let mut raw: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut normalized = Vec::new();
        for end in ends {
            let Some(semantic) = trimmed.get(&end) else { continue; };
            let entries = raw.entry(*semantic).or_insert_with(|| {
                normalized.push(*semantic); Vec::new()
            });
            entries.push(end);
        }
        Self { range: TextRange::new(start, range.end), ends: normalized, raw: Some(raw) }
    }

    pub fn consumed_ends<'a>(&'a self, semantic: &'a usize) -> &'a [usize] {
        match &self.raw {
            None => std::slice::from_ref(semantic),
            Some(raw) => raw.get(semantic).map_or(&[], Vec::as_slice),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_semantic_endpoints_retain_each_raw_end_in_offered_order() {
        let input = "🦀 \t 1  )";
        let start = "🦀 ".len();
        let value = input.find('1').unwrap();
        let ends = vec![value + 3, value + 1, value + 2];
        let normalized = Prefixes::new(input, TextRange::new(start, input.len()), ends.clone());
        assert_eq!(normalized.range.start, value);
        assert_eq!(normalized.ends, [value + 1]);
        assert_eq!(normalized.consumed_ends(&(value + 1)), ends);
    }
    #[test]
    fn java_trim_does_not_create_empty_or_unicode_trimmed_values() {
        let blank = " \t\n";
        let normalized = Prefixes::new(blank, TextRange::new(0, blank.len()), vec![1, blank.len()]);
        assert!(normalized.ends.is_empty());
        let unicode = "\u{a0}x\u{3000}";
        let normalized = Prefixes::new(unicode, TextRange::new(0, unicode.len()), vec![unicode.len()]);
        assert_eq!(normalized.range, TextRange::new(0, unicode.len()));
        assert_eq!(normalized.ends, [unicode.len()]);
    }
}
