use crate::{MappedSource, MappedSpan, TextRange};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawNodeId(u64);

impl RawNodeId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    fn index(self) -> usize {
        usize::try_from(self.0).expect("raw node IDs are allocated from usize indices")
    }
}

impl fmt::Display for RawNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    None,
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndentKind {
    Space,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Indentation {
    pub kind: IndentKind,
    pub unit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawTriviaKind {
    Whitespace,
    LineComment,
    BlockComment,
    LineEnding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTrivia {
    pub kind: RawTriviaKind,
    pub text: String,
    pub span: MappedSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine {
    /// Zero-based physical line number.
    pub number: usize,
    /// Source text excluding the line ending.
    pub raw_text: String,
    pub line_ending: LineEnding,
    /// Full physical line, including its line ending when present.
    pub span: MappedSpan,
    pub content_span: MappedSpan,
    pub line_ending_span: MappedSpan,
    /// Leading whitespace. It is preserved even when indentation is invalid.
    pub indentation: RawTrivia,
    /// Whitespace after code, comments, and the line ending in source order.
    pub trailing_trivia: Vec<RawTrivia>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawNodeKind {
    Blank,
    Comment,
    Simple,
    Section,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RawInvalidReason {
    MixedIndentation,
    InvalidIndentation,
    UnexpectedIndentation {
        expected_level: u32,
        actual_level: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNode {
    pub id: RawNodeId,
    pub kind: RawNodeKind,
    /// Decoded code for Simple/Section/Invalid nodes, or raw comment text.
    pub text: String,
    /// Complete node extent. A Section span includes its descendants.
    pub span: MappedSpan,
    pub line: RawLine,
    /// Code after indentation and before trailing trivia.
    pub code_span: Option<MappedSpan>,
    /// Section header including the trailing colon.
    pub header_span: Option<MappedSpan>,
    /// Section body. Empty sections use a zero-width span after the header line.
    pub body_span: Option<MappedSpan>,
    pub indent_level: Option<u32>,
    pub invalid_reason: Option<RawInvalidReason>,
    pub parent: Option<RawNodeId>,
    pub children: Vec<RawNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawDiagnosticCode {
    MixedIndentation,
    InvalidIndentation,
    UnexpectedIndentation,
    EmptySection,
    UnclosedBlockComment,
}

impl RawDiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MixedIndentation => "mixed-indentation",
            Self::InvalidIndentation => "invalid-indentation",
            Self::UnexpectedIndentation => "unexpected-indentation",
            Self::EmptySection => "empty-section",
            Self::UnclosedBlockComment => "unclosed-block-comment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRelatedSpan {
    pub message: String,
    pub span: MappedSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDiagnostic {
    pub code: RawDiagnosticCode,
    pub severity: RawDiagnosticSeverity,
    pub message: String,
    pub span: MappedSpan,
    pub related: Vec<RawRelatedSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MultilineCommentSupport {
    Unsupported,
    TripleHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawTreeOptions {
    pub multiline_comments: MultilineCommentSupport,
}

impl RawTreeOptions {
    pub const fn new(multiline_comments: MultilineCommentSupport) -> Self {
        Self { multiline_comments }
    }

    /// Selects language features for a Skript release.
    ///
    /// Triple-hash multiline comments were introduced in Skript 2.9.
    pub const fn for_skript_version(major: u32, minor: u32) -> Self {
        let multiline_comments = if major > 2 || (major == 2 && minor >= 9) {
            MultilineCommentSupport::TripleHash
        } else {
            MultilineCommentSupport::Unsupported
        };
        Self::new(multiline_comments)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawTree {
    pub roots: Vec<RawNodeId>,
    pub nodes: Vec<RawNode>,
    pub diagnostics: Vec<RawDiagnostic>,
    pub indentation: Option<Indentation>,
}

impl RawTree {
    pub fn get(&self, id: RawNodeId) -> Option<&RawNode> {
        usize::try_from(id.get())
            .ok()
            .and_then(|index| self.nodes.get(index))
            .filter(|node| node.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RawNode> {
        self.nodes.iter()
    }
}

pub fn parse_raw_tree(source: &MappedSource, options: RawTreeOptions) -> RawTree {
    RawTreeParser::new(source, options).parse()
}

struct RawTreeParser<'a> {
    source: &'a MappedSource,
    options: RawTreeOptions,
    tree: RawTree,
    sections: Vec<OpenSection>,
    in_block_comment: bool,
    block_comment_start: Option<MappedSpan>,
}

#[derive(Debug, Clone, Copy)]
struct OpenSection {
    id: RawNodeId,
    level: u32,
}

impl<'a> RawTreeParser<'a> {
    fn new(source: &'a MappedSource, options: RawTreeOptions) -> Self {
        Self {
            source,
            options,
            tree: RawTree::default(),
            sections: Vec::new(),
            in_block_comment: false,
            block_comment_start: None,
        }
    }

    fn parse(mut self) -> RawTree {
        for physical in physical_lines(self.source.virtual_source()) {
            self.parse_line(physical);
        }

        if let Some(opening) = self.block_comment_start.take() {
            let eof = self.mapped(TextRange::empty(self.source.virtual_source().len()));
            self.tree.diagnostics.push(RawDiagnostic {
                code: RawDiagnosticCode::UnclosedBlockComment,
                severity: RawDiagnosticSeverity::Error,
                message: "block comment opened with ### is not closed".to_owned(),
                span: opening,
                related: vec![RawRelatedSpan {
                    message: "the document ends here".to_owned(),
                    span: eof,
                }],
            });
        }

        self.finalize_sections();
        self.tree.diagnostics.sort_by(|left, right| {
            left.span
                .virtual_range
                .start
                .cmp(&right.span.virtual_range.start)
                .then_with(|| left.code.cmp(&right.code))
        });
        self.tree
    }

    fn parse_line(&mut self, physical: PhysicalLine) {
        let content = physical
            .content_range
            .slice(self.source.virtual_source())
            .expect("physical lines are split at UTF-8 boundaries");
        let indentation_end = leading_whitespace_end(content);
        let split = split_line(
            content,
            self.in_block_comment,
            indentation_end,
            self.options.multiline_comments,
        );

        match split.block_action {
            Some(BlockAction::Open(delimiter)) => {
                self.in_block_comment = true;
                self.block_comment_start =
                    Some(self.mapped(offset_range(physical.content_range.start, delimiter)));
            }
            Some(BlockAction::Close) => {
                self.in_block_comment = false;
                self.block_comment_start = None;
            }
            None => {}
        }

        let line = self.build_line(&physical, content, indentation_end, &split);
        let trimmed = split.value.trim();

        if split.comment.is_some() && trimmed.is_empty() {
            let text = split
                .comment
                .as_ref()
                .map_or("", |comment| &content[comment.start..])
                .to_owned();
            self.push_node(
                RawNodeKind::Comment,
                text,
                line,
                None,
                None,
                self.current_parent(),
            );
            return;
        }

        if trimmed.is_empty() {
            self.push_node(
                RawNodeKind::Blank,
                String::new(),
                line,
                None,
                None,
                self.current_parent(),
            );
            return;
        }

        let indentation = &content[..indentation_end];
        let indentation_span = line.indentation.span.clone();
        match self.resolve_indentation(indentation) {
            Ok(level) => self.parse_code_line(line, trimmed.to_owned(), level),
            Err(reason) => {
                self.push_indentation_diagnostic(&reason, indentation_span);
                self.push_node(
                    RawNodeKind::Invalid,
                    trimmed.to_owned(),
                    line,
                    Some(reason),
                    None,
                    self.current_parent(),
                );
            }
        }
    }

    fn parse_code_line(&mut self, line: RawLine, text: String, level: u32) {
        while self
            .sections
            .last()
            .is_some_and(|section| section.level >= level)
        {
            self.sections.pop();
        }

        let expected_level = self.sections.last().map_or(0, |section| section.level + 1);
        if level != expected_level {
            let reason = RawInvalidReason::UnexpectedIndentation {
                expected_level,
                actual_level: level,
            };
            let diagnostic_span = if line.indentation.span.virtual_range.is_empty() {
                line.code_span(self.source)
                    .unwrap_or_else(|| line.content_span.clone())
            } else {
                line.indentation.span.clone()
            };
            self.push_indentation_diagnostic(&reason, diagnostic_span);
            self.push_node(
                RawNodeKind::Invalid,
                text,
                line,
                Some(reason),
                Some(level),
                self.current_parent(),
            );
            return;
        }

        let parent = self.current_parent();
        if let Some(header) = text.strip_suffix(':') {
            let id = self.push_node(
                RawNodeKind::Section,
                header.to_owned(),
                line,
                None,
                Some(level),
                parent,
            );
            self.sections.push(OpenSection { id, level });
        } else {
            self.push_node(RawNodeKind::Simple, text, line, None, Some(level), parent);
        }
    }

    fn build_line(
        &self,
        physical: &PhysicalLine,
        content: &str,
        indentation_end: usize,
        split: &LineSplit,
    ) -> RawLine {
        let content_start = physical.content_range.start;
        let comment_start = split
            .comment
            .as_ref()
            .map_or(content.len(), |comment| comment.start);
        let value = &content[..comment_start];
        let code_end = trim_end_whitespace(value);
        let has_code = !split.value.trim().is_empty();
        let mut trailing_trivia = Vec::new();

        if has_code && code_end < comment_start {
            let range = TextRange::new(content_start + code_end, content_start + comment_start);
            trailing_trivia.push(RawTrivia {
                kind: RawTriviaKind::Whitespace,
                text: range
                    .slice(self.source.virtual_source())
                    .expect("trailing whitespace is a valid range")
                    .to_owned(),
                span: self.mapped(range),
            });
        }

        if let Some(comment) = &split.comment {
            let range = TextRange::new(content_start + comment.start, physical.content_range.end);
            trailing_trivia.push(RawTrivia {
                kind: match comment.kind {
                    CommentKind::Line => RawTriviaKind::LineComment,
                    CommentKind::Block => RawTriviaKind::BlockComment,
                },
                text: range
                    .slice(self.source.virtual_source())
                    .expect("comment is a valid range")
                    .to_owned(),
                span: self.mapped(range),
            });
        }

        if physical.line_ending != LineEnding::None {
            trailing_trivia.push(RawTrivia {
                kind: RawTriviaKind::LineEnding,
                text: physical.line_ending.as_str().to_owned(),
                span: self.mapped(physical.line_ending_range),
            });
        }

        let indentation_range = TextRange::new(content_start, content_start + indentation_end);
        RawLine {
            number: physical.number,
            raw_text: content.to_owned(),
            line_ending: physical.line_ending,
            span: self.mapped(physical.full_range),
            content_span: self.mapped(physical.content_range),
            line_ending_span: self.mapped(physical.line_ending_range),
            indentation: RawTrivia {
                kind: RawTriviaKind::Whitespace,
                text: content[..indentation_end].to_owned(),
                span: self.mapped(indentation_range),
            },
            trailing_trivia,
        }
    }

    fn resolve_indentation(&mut self, indentation: &str) -> Result<u32, RawInvalidReason> {
        if indentation.is_empty() {
            return Ok(0);
        }

        let kind = indentation_kind(indentation)?;
        if let Some(style) = &self.tree.indentation {
            if style.kind != kind || !indentation.len().is_multiple_of(style.unit.len()) {
                return Err(RawInvalidReason::InvalidIndentation);
            }
            let level = indentation.len() / style.unit.len();
            return u32::try_from(level).map_err(|_| RawInvalidReason::InvalidIndentation);
        }

        if self.sections.is_empty() {
            return Err(RawInvalidReason::UnexpectedIndentation {
                expected_level: 0,
                actual_level: 1,
            });
        }

        self.tree.indentation = Some(Indentation {
            kind,
            unit: indentation.to_owned(),
        });
        Ok(1)
    }

    fn push_indentation_diagnostic(&mut self, reason: &RawInvalidReason, span: MappedSpan) {
        let (code, message) = match reason {
            RawInvalidReason::MixedIndentation => (
                RawDiagnosticCode::MixedIndentation,
                "indentation must not mix spaces and tabs".to_owned(),
            ),
            RawInvalidReason::InvalidIndentation => {
                let expected = self.tree.indentation.as_ref().map_or_else(
                    || "only spaces or only tabs".to_owned(),
                    |indentation| format!("repetitions of {:?}", indentation.unit),
                );
                (
                    RawDiagnosticCode::InvalidIndentation,
                    format!("indentation must use {expected}"),
                )
            }
            RawInvalidReason::UnexpectedIndentation {
                expected_level,
                actual_level,
            } => (
                RawDiagnosticCode::UnexpectedIndentation,
                format!("expected indentation level {expected_level}, found level {actual_level}"),
            ),
        };
        self.tree.diagnostics.push(RawDiagnostic {
            code,
            severity: RawDiagnosticSeverity::Error,
            message,
            span,
            related: Vec::new(),
        });
    }

    fn push_node(
        &mut self,
        kind: RawNodeKind,
        text: String,
        line: RawLine,
        invalid_reason: Option<RawInvalidReason>,
        indent_level: Option<u32>,
        parent: Option<RawNodeId>,
    ) -> RawNodeId {
        let id = RawNodeId::new(
            u64::try_from(self.tree.nodes.len()).expect("raw tree node count exceeds u64"),
        );
        let code_span = line.code_span(self.source);
        let header_span = (kind == RawNodeKind::Section)
            .then(|| code_span.clone())
            .flatten();
        let node = RawNode {
            id,
            kind,
            text,
            span: line.span.clone(),
            line,
            code_span,
            header_span,
            body_span: None,
            indent_level,
            invalid_reason,
            parent,
            children: Vec::new(),
        };

        if let Some(parent) = parent {
            self.tree.nodes[parent.index()].children.push(id);
        } else {
            self.tree.roots.push(id);
        }
        self.tree.nodes.push(node);
        id
    }

    fn current_parent(&self) -> Option<RawNodeId> {
        self.sections.last().map(|section| section.id)
    }

    fn finalize_sections(&mut self) {
        let section_ids = self
            .tree
            .nodes
            .iter()
            .filter(|node| node.kind == RawNodeKind::Section)
            .map(|node| node.id)
            .collect::<Vec<_>>();

        for id in section_ids.into_iter().rev() {
            let index = id.index();
            let children = self.tree.nodes[index].children.clone();
            let line_start = self.tree.nodes[index].line.span.virtual_range.start;
            let line_end = self.tree.nodes[index].line.span.virtual_range.end;
            let body_range = children
                .first()
                .map_or(TextRange::empty(line_end), |first| {
                    let start = self.tree.nodes[first.index()].line.span.virtual_range.start;
                    let last = children
                        .last()
                        .expect("a non-empty child list has a last item");
                    let end = self.tree.nodes[last.index()].span.virtual_range.end;
                    TextRange::new(start, end)
                });
            let span = self.mapped(TextRange::new(line_start, body_range.end));
            let body_span = self.mapped(body_range);
            let empty = children.iter().all(|child| {
                matches!(
                    self.tree.nodes[child.index()].kind,
                    RawNodeKind::Blank | RawNodeKind::Comment
                )
            });

            self.tree.nodes[index].span = span;
            self.tree.nodes[index].body_span = Some(body_span);

            if empty {
                self.tree.diagnostics.push(RawDiagnostic {
                    code: RawDiagnosticCode::EmptySection,
                    severity: RawDiagnosticSeverity::Warning,
                    message: "section has no non-trivia body nodes".to_owned(),
                    span: self.tree.nodes[index]
                        .header_span
                        .clone()
                        .expect("section nodes have a header span"),
                    related: Vec::new(),
                });
            }
        }
    }

    fn mapped(&self, range: TextRange) -> MappedSpan {
        self.source
            .map_range(range)
            .expect("raw tree ranges are built from the mapped virtual source")
    }
}

impl RawLine {
    fn code_span(&self, source: &MappedSource) -> Option<MappedSpan> {
        let indentation_end = self.indentation.span.virtual_range.end;
        let trailing_start = self
            .trailing_trivia
            .iter()
            .find(|trivia| trivia.kind != RawTriviaKind::LineEnding)
            .map_or(self.content_span.virtual_range.end, |trivia| {
                trivia.span.virtual_range.start
            });
        (indentation_end < trailing_start).then(|| {
            source
                .map_range(TextRange::new(indentation_end, trailing_start))
                .expect("code range is within the virtual source")
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct PhysicalLine {
    number: usize,
    content_range: TextRange,
    line_ending_range: TextRange,
    full_range: TextRange,
    line_ending: LineEnding,
}

fn physical_lines(source: &str) -> Vec<PhysicalLine> {
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut number = 0;
    let mut cursor = 0;

    while cursor < bytes.len() {
        let (ending, ending_len) = match bytes[cursor] {
            b'\n' => (LineEnding::Lf, 1),
            b'\r' if bytes.get(cursor + 1) == Some(&b'\n') => (LineEnding::CrLf, 2),
            b'\r' => (LineEnding::Cr, 1),
            _ => {
                cursor += 1;
                continue;
            }
        };
        let end = cursor + ending_len;
        lines.push(PhysicalLine {
            number,
            content_range: TextRange::new(start, cursor),
            line_ending_range: TextRange::new(cursor, end),
            full_range: TextRange::new(start, end),
            line_ending: ending,
        });
        number += 1;
        start = end;
        cursor = end;
    }

    if start < source.len() {
        lines.push(PhysicalLine {
            number,
            content_range: TextRange::new(start, source.len()),
            line_ending_range: TextRange::empty(source.len()),
            full_range: TextRange::new(start, source.len()),
            line_ending: LineEnding::None,
        });
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentKind {
    Line,
    Block,
}

#[derive(Debug, Clone)]
struct CommentSplit {
    start: usize,
    kind: CommentKind,
}

#[derive(Debug, Clone, Copy)]
enum BlockAction {
    Open(TextRange),
    Close,
}

#[derive(Debug, Clone)]
struct LineSplit {
    value: String,
    comment: Option<CommentSplit>,
    block_action: Option<BlockAction>,
}

fn split_line(
    line: &str,
    in_block_comment: bool,
    indentation_end: usize,
    multiline_comments: MultilineCommentSupport,
) -> LineSplit {
    let trim_start = leading_whitespace_end(line);
    let trim_end = trim_end_whitespace(line);
    let trimmed = if trim_start < trim_end {
        &line[trim_start..trim_end]
    } else {
        ""
    };

    if multiline_comments == MultilineCommentSupport::TripleHash && trimmed == "###" {
        return LineSplit {
            value: String::new(),
            comment: Some(CommentSplit {
                start: indentation_end,
                kind: CommentKind::Block,
            }),
            block_action: Some(if in_block_comment {
                BlockAction::Close
            } else {
                BlockAction::Open(TextRange::new(trim_start, trim_start + 3))
            }),
        };
    }
    if in_block_comment {
        return LineSplit {
            value: String::new(),
            comment: Some(CommentSplit {
                start: indentation_end,
                kind: CommentKind::Block,
            }),
            block_action: None,
        };
    }
    if trimmed.starts_with('#') {
        return LineSplit {
            value: String::new(),
            comment: Some(CommentSplit {
                start: trim_start,
                kind: CommentKind::Line,
            }),
            block_action: None,
        };
    }

    split_inline_comment(line)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitState {
    Code,
    String,
    Variable,
}

fn split_inline_comment(line: &str) -> LineSplit {
    let mut value = String::with_capacity(line.len());
    let mut state = SplitState::Code;
    let mut previous_state = SplitState::Code;
    let mut chars = line.char_indices().peekable();

    while let Some((offset, character)) = chars.next() {
        if matches!(character, '%' | '"' | '#')
            && chars.peek().is_some_and(|(_, next)| *next == character)
            && (character != '#' || state != SplitState::String)
        {
            chars.next();
            if character == '#' {
                value.push('#');
            } else {
                value.push(character);
                value.push(character);
            }
            continue;
        }

        if character == '#' && state != SplitState::String {
            return LineSplit {
                value,
                comment: Some(CommentSplit {
                    start: offset,
                    kind: CommentKind::Line,
                }),
                block_action: None,
            };
        }

        value.push(character);
        match character {
            '%' => {
                let old_state = state;
                state = if state == SplitState::Code {
                    previous_state
                } else {
                    SplitState::Code
                };
                if state == SplitState::Code {
                    previous_state = old_state;
                }
            }
            '"' => {
                state = match state {
                    SplitState::Code => SplitState::String,
                    SplitState::String => SplitState::Code,
                    SplitState::Variable => SplitState::Variable,
                };
            }
            '{' if state != SplitState::String => state = SplitState::Variable,
            '}' if state != SplitState::String => state = SplitState::Code,
            _ => {}
        }
    }

    LineSplit {
        value,
        comment: None,
        block_action: None,
    }
}

fn indentation_kind(indentation: &str) -> Result<IndentKind, RawInvalidReason> {
    let only_spaces = indentation.bytes().all(|byte| byte == b' ');
    let only_tabs = indentation.bytes().all(|byte| byte == b'\t');
    if only_spaces {
        Ok(IndentKind::Space)
    } else if only_tabs {
        Ok(IndentKind::Tab)
    } else if indentation.contains(' ') && indentation.contains('\t') {
        Err(RawInvalidReason::MixedIndentation)
    } else {
        Err(RawInvalidReason::InvalidIndentation)
    }
}

fn leading_whitespace_end(text: &str) -> usize {
    text.char_indices()
        .find_map(|(offset, character)| (!character.is_whitespace()).then_some(offset))
        .unwrap_or(text.len())
}

fn trim_end_whitespace(text: &str) -> usize {
    text.char_indices()
        .rev()
        .find_map(|(offset, character)| {
            (!character.is_whitespace()).then_some(offset + character.len_utf8())
        })
        .unwrap_or(0)
}

fn offset_range(base: usize, range: TextRange) -> TextRange {
    TextRange::new(base + range.start, base + range.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceOrigin;
    use proptest::prelude::*;

    fn parse(source: &str) -> RawTree {
        parse_version(source, 2, 9)
    }

    fn parse_version(source: &str, major: u32, minor: u32) -> RawTree {
        parse_raw_tree(
            &MappedSource::identity(source),
            RawTreeOptions::for_skript_version(major, minor),
        )
    }

    fn node(tree: &RawTree, id: RawNodeId) -> &RawNode {
        tree.get(id).expect("test node must exist")
    }

    #[test]
    fn splits_physical_lines_losslessly() {
        let source = "first\r\n\n日本語";
        let tree = parse(source);

        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(
            tree.nodes.iter().map(|node| node.kind).collect::<Vec<_>>(),
            [RawNodeKind::Simple, RawNodeKind::Blank, RawNodeKind::Simple]
        );
        assert_eq!(tree.nodes[0].line.line_ending, LineEnding::CrLf);
        assert_eq!(tree.nodes[1].line.line_ending, LineEnding::Lf);
        assert_eq!(tree.nodes[2].line.line_ending, LineEnding::None);
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| { format!("{}{}", node.line.raw_text, node.line.line_ending.as_str()) })
                .collect::<String>(),
            source
        );
        assert_eq!(
            tree.nodes[2].line.content_span.primary_origin(),
            Some(SourceOrigin::exact(TextRange::new(8, 17), None))
        );

        let trailing_newline = parse("line\n");
        assert_eq!(trailing_newline.nodes.len(), 1);
        assert_eq!(trailing_newline.nodes[0].line.line_ending, LineEnding::Lf);

        let blank_line = parse("\n");
        assert_eq!(blank_line.nodes.len(), 1);
        assert_eq!(blank_line.nodes[0].kind, RawNodeKind::Blank);
    }

    #[test]
    fn matches_skript_comment_splitting_rules() {
        let cases = [
            ("", "", None),
            ("ab", "ab", None),
            ("ab#", "ab", Some("#")),
            ("ab##", "ab#", None),
            ("ab###", "ab#", Some("#")),
            ("#ab", "", Some("#ab")),
            ("ab#cd", "ab", Some("#cd")),
            ("ab##cd", "ab#cd", None),
            ("ab###cd", "ab#", Some("#cd")),
            ("######", "", Some("######")),
            ("#######", "", Some("#######")),
            ("#### # ####", "", Some("#### # ####")),
            ("##### ####", "", Some("##### ####")),
            ("#### #####", "", Some("#### #####")),
            ("#########", "", Some("#########")),
            ("a##b#c##d#e", "a#b", Some("#c##d#e")),
            (" a ## b # c ## d # e ", " a # b ", Some("# c ## d # e ")),
            ("a b \"#a  ##\" # b \"", "a b \"#a  ##\" ", Some("# b \"")),
        ];

        for (input, expected_value, expected_comment) in cases {
            let split = split_line(
                input,
                false,
                leading_whitespace_end(input),
                MultilineCommentSupport::TripleHash,
            );
            assert_eq!(split.value, expected_value, "{input:?}");
            assert_eq!(
                split.comment.map(|comment| &input[comment.start..]),
                expected_comment,
                "{input:?}"
            );
        }
    }

    #[test]
    fn preserves_comments_escaped_hashes_and_trivia() {
        let source =
            "on load: # header\n    set {_x##} to \"##\" # tail\n    ###\n    ignored\n    ###\n";
        let tree = parse(source);
        let section = node(&tree, tree.roots[0]);

        assert_eq!(section.kind, RawNodeKind::Section);
        assert_eq!(section.text, "on load");
        assert_eq!(section.children.len(), 4);
        let simple = node(&tree, section.children[0]);
        assert_eq!(simple.text, "set {_x#} to \"##\"");
        assert_eq!(
            simple
                .line
                .trailing_trivia
                .iter()
                .map(|trivia| trivia.kind)
                .collect::<Vec<_>>(),
            [
                RawTriviaKind::Whitespace,
                RawTriviaKind::LineComment,
                RawTriviaKind::LineEnding
            ]
        );
        assert_eq!(
            section.children[1..]
                .iter()
                .map(|id| node(&tree, *id).kind)
                .collect::<Vec<_>>(),
            [
                RawNodeKind::Comment,
                RawNodeKind::Comment,
                RawNodeKind::Comment
            ]
        );
        assert!(tree.diagnostics.is_empty());
    }

    #[test]
    fn selects_multiline_comment_support_from_the_skript_version() {
        assert_eq!(
            RawTreeOptions::for_skript_version(2, 8).multiline_comments,
            MultilineCommentSupport::Unsupported
        );
        assert_eq!(
            RawTreeOptions::for_skript_version(2, 9).multiline_comments,
            MultilineCommentSupport::TripleHash
        );
        assert_eq!(
            RawTreeOptions::for_skript_version(3, 0).multiline_comments,
            MultilineCommentSupport::TripleHash
        );
    }

    #[test]
    fn triple_hash_lines_are_version_gated() {
        let source = "before\n###\ninside\n###\nafter\n";
        let legacy = parse_version(source, 2, 8);
        let modern = parse_version(source, 2, 9);

        assert_eq!(legacy.nodes[2].kind, RawNodeKind::Simple);
        assert_eq!(legacy.nodes[2].text, "inside");
        assert_eq!(
            legacy.nodes[1].line.trailing_trivia[0].kind,
            RawTriviaKind::LineComment
        );
        assert!(legacy.diagnostics.is_empty());

        assert_eq!(modern.nodes[2].kind, RawNodeKind::Comment);
        assert_eq!(
            modern.nodes[1].line.trailing_trivia[0].kind,
            RawTriviaKind::BlockComment
        );
        assert!(modern.diagnostics.is_empty());
    }

    #[test]
    fn triple_hashes_in_the_middle_of_a_line_never_toggle_block_comments() {
        for version in [(2, 8), (2, 9)] {
            let tree = parse_version("hello ### there\nnext\n", version.0, version.1);

            assert_eq!(tree.nodes[0].kind, RawNodeKind::Simple);
            assert_eq!(tree.nodes[0].text, "hello #");
            assert_eq!(tree.nodes[1].kind, RawNodeKind::Simple);
            assert_eq!(tree.nodes[1].text, "next");
            assert!(
                tree.nodes[0]
                    .line
                    .trailing_trivia
                    .iter()
                    .any(|trivia| trivia.kind == RawTriviaKind::LineComment)
            );
            assert!(tree.diagnostics.is_empty());
        }
    }

    #[test]
    fn block_comment_interior_keeps_block_trivia_kind() {
        let source = "###\n### still inside\n# also inside\n###\nsend \"done\"\n";
        let tree = parse(source);

        assert_eq!(
            tree.nodes[..4]
                .iter()
                .map(|node| {
                    node.line
                        .trailing_trivia
                        .iter()
                        .find(|trivia| {
                            matches!(
                                trivia.kind,
                                RawTriviaKind::LineComment | RawTriviaKind::BlockComment
                            )
                        })
                        .map(|trivia| trivia.kind)
                })
                .collect::<Vec<_>>(),
            [Some(RawTriviaKind::BlockComment); 4]
        );
        assert_eq!(tree.nodes[4].kind, RawNodeKind::Simple);
        assert!(tree.diagnostics.is_empty());
    }

    #[test]
    fn builds_nested_space_indented_sections_and_spans() {
        let source = "on load:\n    send \"a\"\n    if true:\n        send \"b\"\nsend \"done\"";
        let tree = parse(source);

        assert_eq!(
            tree.indentation,
            Some(Indentation {
                kind: IndentKind::Space,
                unit: "    ".to_owned()
            })
        );
        assert_eq!(tree.roots.len(), 2);
        let outer = node(&tree, tree.roots[0]);
        let root_simple = node(&tree, tree.roots[1]);
        assert_eq!(outer.kind, RawNodeKind::Section);
        assert_eq!(outer.indent_level, Some(0));
        assert_eq!(outer.children.len(), 2);
        assert_eq!(root_simple.parent, None);
        assert_eq!(root_simple.text, "send \"done\"");

        let nested = node(&tree, outer.children[1]);
        assert_eq!(nested.kind, RawNodeKind::Section);
        assert_eq!(nested.parent, Some(outer.id));
        assert_eq!(nested.indent_level, Some(1));
        assert_eq!(node(&tree, nested.children[0]).indent_level, Some(2));
        assert_eq!(
            nested
                .header_span
                .as_ref()
                .and_then(|span| span.virtual_range.slice(source)),
            Some("if true:")
        );
        assert_eq!(
            nested
                .body_span
                .as_ref()
                .and_then(|span| span.virtual_range.slice(source)),
            Some("        send \"b\"\n")
        );
        assert_eq!(
            outer.span.virtual_range.slice(source),
            Some("on load:\n    send \"a\"\n    if true:\n        send \"b\"\n")
        );
        assert!(tree.diagnostics.is_empty());
    }

    #[test]
    fn supports_tab_indentation() {
        let tree = parse("on load:\n\tsend \"a\"\n\tif true:\n\t\tsend \"b\"\n");

        assert_eq!(
            tree.indentation,
            Some(Indentation {
                kind: IndentKind::Tab,
                unit: "\t".to_owned()
            })
        );
        assert_eq!(tree.nodes[3].indent_level, Some(2));
        assert!(tree.diagnostics.is_empty());
    }

    #[test]
    fn recovers_after_mixed_over_and_partial_indentation() {
        let source = concat!(
            "on load:\n",
            " \tbroken mixed\n",
            "    send \"recovered\"\n",
            "        broken too deep\n",
            "  broken partial\n",
            "    send \"after\"\n",
            "send \"root\"\n",
        );
        let tree = parse(source);

        assert_eq!(
            tree.nodes.iter().map(|node| node.kind).collect::<Vec<_>>(),
            [
                RawNodeKind::Section,
                RawNodeKind::Invalid,
                RawNodeKind::Simple,
                RawNodeKind::Invalid,
                RawNodeKind::Invalid,
                RawNodeKind::Simple,
                RawNodeKind::Simple,
            ]
        );
        assert_eq!(
            tree.diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            [
                RawDiagnosticCode::MixedIndentation,
                RawDiagnosticCode::UnexpectedIndentation,
                RawDiagnosticCode::InvalidIndentation,
            ]
        );
        assert_eq!(tree.nodes[5].parent, Some(tree.nodes[0].id));
        assert_eq!(tree.nodes[6].parent, None);
    }

    #[test]
    fn comments_ignore_indentation_and_empty_sections_warn() {
        let source = "on load:\n# still belongs to the section\n\nsend \"root\"\n";
        let tree = parse(source);
        let section = node(&tree, tree.roots[0]);

        assert_eq!(section.children.len(), 2);
        assert_eq!(node(&tree, section.children[0]).kind, RawNodeKind::Comment);
        assert_eq!(node(&tree, section.children[1]).kind, RawNodeKind::Blank);
        assert_eq!(tree.roots.len(), 2);
        assert_eq!(tree.diagnostics.len(), 1);
        assert_eq!(tree.diagnostics[0].code, RawDiagnosticCode::EmptySection);
        assert_eq!(
            section
                .body_span
                .as_ref()
                .and_then(|span| span.virtual_range.slice(source)),
            Some("# still belongs to the section\n\n")
        );
    }

    #[test]
    fn empty_section_without_a_final_newline_has_an_empty_body_span() {
        let source = "on load:";
        let tree = parse(source);
        let section = &tree.nodes[0];

        assert_eq!(section.kind, RawNodeKind::Section);
        assert_eq!(
            section.body_span.as_ref().unwrap().virtual_range,
            TextRange::empty(source.len())
        );
        assert_eq!(tree.diagnostics[0].code, RawDiagnosticCode::EmptySection);
    }

    #[test]
    fn legacy_triple_hashes_do_not_open_an_unclosed_block_comment() {
        let tree = parse_version("###\ncode\n", 2, 8);

        assert_eq!(tree.nodes[0].kind, RawNodeKind::Comment);
        assert_eq!(tree.nodes[1].kind, RawNodeKind::Simple);
        assert!(tree.diagnostics.is_empty());
    }

    #[test]
    fn reports_unclosed_block_comments_at_the_opening_marker() {
        let source = "###\ncomment\n";
        let tree = parse(source);

        assert_eq!(tree.nodes.len(), 2);
        assert!(
            tree.nodes
                .iter()
                .all(|node| node.kind == RawNodeKind::Comment)
        );
        assert_eq!(tree.diagnostics.len(), 1);
        let diagnostic = &tree.diagnostics[0];
        assert_eq!(diagnostic.code, RawDiagnosticCode::UnclosedBlockComment);
        assert_eq!(diagnostic.span.virtual_range.slice(source), Some("###"));
        assert_eq!(
            diagnostic.related[0].span.virtual_range,
            TextRange::empty(source.len())
        );
    }

    #[test]
    fn empty_documents_have_no_physical_nodes() {
        let tree = parse("");
        assert!(tree.nodes.is_empty());
        assert!(tree.roots.is_empty());
        assert!(tree.diagnostics.is_empty());
    }

    #[test]
    fn arbitrary_node_ids_are_safe_to_query() {
        let tree = parse("send \"hello\"");

        assert_eq!(tree.get(RawNodeId::new(0)), tree.nodes.first());
        assert_eq!(tree.get(RawNodeId::new(u64::MAX)), None);
    }

    #[test]
    fn mapped_sources_keep_macro_origins_on_raw_nodes() {
        let expanded = MappedSource::identity("abc")
            .apply_text_edits(
                [
                    crate::TextEdit::new(TextRange::new(0, 1), "A"),
                    crate::TextEdit::new(TextRange::new(2, 3), "C"),
                ],
                crate::TextExpansion::new("test.component", "expand"),
            )
            .unwrap();
        let tree = parse_raw_tree(&expanded.source, RawTreeOptions::for_skript_version(2, 9));

        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].line.raw_text, "AbC");
        assert_eq!(
            tree.nodes[0]
                .line
                .span
                .origins
                .iter()
                .map(|origin| origin.original_range)
                .collect::<Vec<_>>(),
            [
                TextRange::new(0, 1),
                TextRange::new(1, 2),
                TextRange::new(2, 3),
            ]
        );
    }

    proptest! {
        #[test]
        fn arbitrary_utf8_sources_are_lossless_and_have_valid_spans(
            chars in proptest::collection::vec(any::<char>(), 0..200)
        ) {
            let source = chars.into_iter().collect::<String>();
            let mapped = MappedSource::identity(source.clone());
            let tree = parse_raw_tree(
                &mapped,
                RawTreeOptions::for_skript_version(2, 9),
            );
            let reconstructed = tree
                .nodes
                .iter()
                .map(|node| {
                    format!(
                        "{}{}",
                        node.line.raw_text,
                        node.line.line_ending.as_str()
                    )
                })
                .collect::<String>();

            prop_assert_eq!(reconstructed, source.clone());
            for node in &tree.nodes {
                prop_assert!(node.span.virtual_range.is_valid_for(&source));
                prop_assert!(node.line.span.virtual_range.is_valid_for(&source));
                prop_assert!(node.line.content_span.virtual_range.is_valid_for(&source));
                prop_assert!(node.line.line_ending_span.virtual_range.is_valid_for(&source));
                prop_assert!(node.line.indentation.span.virtual_range.is_valid_for(&source));
                for trivia in &node.line.trailing_trivia {
                    prop_assert!(trivia.span.virtual_range.is_valid_for(&source));
                }
                if let Some(parent) = node.parent {
                    prop_assert!(tree.get(parent).is_some());
                }
                for child in &node.children {
                    prop_assert_eq!(tree.get(*child).and_then(|node| node.parent), Some(node.id));
                }
            }
        }
    }
}
