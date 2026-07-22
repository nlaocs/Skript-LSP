use std::fmt;

/// A half-open UTF-8 byte range in a source text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn empty(offset: usize) -> Self {
        Self::new(offset, offset)
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn is_valid_for(self, source: &str) -> bool {
        self.start <= self.end
            && self.end <= source.len()
            && source.is_char_boundary(self.start)
            && source.is_char_boundary(self.end)
    }

    pub fn slice(self, source: &str) -> Option<&str> {
        source.get(self.start..self.end)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    pub(crate) fn intersection(self, other: Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start < end).then(|| Self::new(start, end))
    }
}

impl fmt::Display for TextRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_utf8_byte_boundaries() {
        let source = "a日本語🙂z";
        assert!(TextRange::new(1, 10).is_valid_for(source));
        assert_eq!(TextRange::new(1, 10).slice(source), Some("日本語"));
        assert!(!TextRange::new(2, 10).is_valid_for(source));
        assert!(!TextRange::new(10, 9).is_valid_for(source));
    }
}
