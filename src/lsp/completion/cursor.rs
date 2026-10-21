//! Shared editing frames for quotes, variable names and interpolation.
use super::Lexical;

pub(super) struct Event {
    pub offset: usize,
    pub character: char,
    pub state: Lexical,
    pub depth: usize,
    pub after_depth: usize,
}
pub(super) struct LexicalCursor<'a> {
    text: &'a str,
    offset: usize,
    frames: Vec<Lexical>,
}
impl<'a> LexicalCursor<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            offset: 0,
            frames: vec![Lexical::Code(false)],
        }
    }
    pub fn state(&self) -> Lexical {
        *self.frames.last().unwrap()
    }
    pub fn depth(&self) -> usize {
        self.frames.len()
    }
}
impl Iterator for LexicalCursor<'_> {
    type Item = Event;
    fn next(&mut self) -> Option<Event> {
        if self.offset == self.text.len() || matches!(self.state(), Lexical::Comment) {
            return None;
        }
        let offset = self.offset;
        let character = self.text[offset..].chars().next()?;
        let state = self.state();
        let depth = self.frames.len();
        let doubled = self.text.as_bytes().get(offset + 1) == Some(&(character as u8));
        let mut advance = character.len_utf8();
        match state {
            Lexical::Code(interpolation) => match character {
                '%' if interpolation => {
                    self.frames.pop();
                }
                '"' => self.frames.push(Lexical::String),
                '{' => self.frames.push(Lexical::Variable(offset)),
                '#' if !interpolation => *self.frames.last_mut().unwrap() = Lexical::Comment,
                _ => {}
            },
            Lexical::String => match character {
                '"' | '%' if doubled => advance += 1,
                '"' => {
                    self.frames.pop();
                }
                '%' => self.frames.push(Lexical::Code(true)),
                _ => {}
            },
            Lexical::Variable(start) => match character {
                // Nested braces remain part of the one public variable name.
                '{' => self.frames.push(Lexical::Variable(start)),
                '}' => {
                    self.frames.pop();
                }
                '%' if doubled => advance += 1,
                '%' => self.frames.push(Lexical::Code(true)),
                _ => {}
            },
            Lexical::Comment => return None,
        }
        self.offset += advance;
        Some(Event {
            offset,
            character,
            state,
            depth,
            after_depth: self.frames.len(),
        })
    }
}

/// A lexical edit boundary, not a claim about a variable's value. Closed
/// names keep the full right suffix for the shared exact-suffix adapter.
/// With no closing brace, opaque syntax to the right is not safely replaceable;
/// nested syntax already in the typed prefix may still be completed at EOL.
pub(super) fn variable_edit_end(text: &str, cursor: usize) -> Result<Option<usize>, ()> {
    if !text.starts_with('{') {
        return Err(());
    }
    let mut opaque_suffix = false;
    for event in LexicalCursor::new(text) {
        if event.offset == 0 {
            continue;
        }
        if event.offset >= cursor
            && (event.character == '{' || event.character == '%' && event.after_depth > event.depth)
        {
            opaque_suffix = true;
        }
        if event.character == '}' && event.after_depth == 1 {
            return Ok(Some(event.offset + 1));
        }
    }
    if opaque_suffix { Err(()) } else { Ok(None) }
}
