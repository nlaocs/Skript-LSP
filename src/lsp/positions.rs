use lsp_types::{Position, Range};
use skript_parser::TextRange;

/// Byte line starts for one immutable original source revision.
pub(super) struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        );
        Self { starts }
    }

    pub fn position(&self, source: &str, offset: usize) -> Result<Position, String> {
        if offset > source.len() || !source.is_char_boundary(offset) {
            return Err(format!("invalid original UTF-8 offset {offset}"));
        }
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let prefix = &source[self.starts[line]..offset];
        Ok(Position::new(
            u32::try_from(line).map_err(|_| "too many lines")?,
            u32::try_from(prefix.trim_end_matches('\r').encode_utf16().count())
                .map_err(|_| "line is too long")?,
        ))
    }

    pub fn range(&self, source: &str, range: TextRange) -> Result<Range, String> {
        if range.start > range.end {
            return Err("reversed original range".into());
        }
        Ok(Range::new(
            self.position(source, range.start)?,
            self.position(source, range.end)?,
        ))
    }

    /// Rejects out-of-line columns and positions inside a UTF-16 surrogate pair.
    pub fn offset(&self, source: &str, position: Position) -> Option<usize> {
        let line = usize::try_from(position.line).ok()?;
        let start = *self.starts.get(line)?;
        let end = self.starts.get(line + 1).copied().unwrap_or(source.len());
        let text = source
            .get(start..end)?
            .trim_end_matches('\n')
            .trim_end_matches('\r');
        let mut units = 0;
        for (byte, character) in text.char_indices() {
            if units == position.character {
                return Some(start + byte);
            }
            units += u32::try_from(character.len_utf16()).ok()?;
            if units > position.character {
                return None;
            }
        }
        (units == position.character).then_some(start + text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_positions_reject_surrogate_interiors_and_out_of_line_columns() {
        let source = "日本😀x\r\n\r\n終";
        let index = LineIndex::new(source);
        for byte in [0, 3, 6, 10, 11, 13, 15, 18] {
            let position = index.position(source, byte).unwrap();
            assert_eq!(index.offset(source, position), Some(byte));
        }
        assert_eq!(index.offset(source, Position::new(0, 3)), None);
        assert_eq!(index.offset(source, Position::new(0, 6)), None);
        assert_eq!(index.offset(source, Position::new(1, 1)), None);
        assert_eq!(index.offset(source, Position::new(3, 0)), None);
        assert_eq!(LineIndex::new("").offset("", Position::new(0, 0)), Some(0));
    }
}
