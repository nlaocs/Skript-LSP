use crate::{
    ComponentId, Expansion, ExpansionGraph, ExpansionGraphError, ExpansionId, ExpansionKind,
    ExpansionSite, HookId, SyntaxContextId, TextRange,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginKind {
    /// The virtual text is byte-for-byte identical to the original text.
    Exact,
    /// The virtual text replaces the complete original range.
    Replaced,
    /// The virtual text was generated at a zero-width original position.
    Anchored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceOrigin {
    pub original_range: TextRange,
    pub kind: OriginKind,
    pub expansion: Option<ExpansionId>,
}

impl SourceOrigin {
    pub const fn exact(original_range: TextRange, expansion: Option<ExpansionId>) -> Self {
        Self {
            original_range,
            kind: OriginKind::Exact,
            expansion,
        }
    }

    pub const fn replaced(original_range: TextRange, expansion: Option<ExpansionId>) -> Self {
        Self {
            original_range,
            kind: OriginKind::Replaced,
            expansion,
        }
    }

    pub const fn anchored(original_offset: usize, expansion: ExpansionId) -> Self {
        Self {
            original_range: TextRange::empty(original_offset),
            kind: OriginKind::Anchored,
            expansion: Some(expansion),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceMapSegment {
    pub virtual_range: TextRange,
    pub origin: SourceOrigin,
}

impl SourceMapSegment {
    pub const fn new(virtual_range: TextRange, origin: SourceOrigin) -> Self {
        Self {
            virtual_range,
            origin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    original_len: usize,
    virtual_len: usize,
    segments: Vec<SourceMapSegment>,
}

impl SourceMap {
    fn new(
        original: &str,
        virtual_source: &str,
        segments: Vec<SourceMapSegment>,
        expansions: &ExpansionGraph,
    ) -> Result<Self, SourceMapError> {
        Self::validate_segments(original, virtual_source, &segments, expansions)?;
        Ok(Self {
            original_len: original.len(),
            virtual_len: virtual_source.len(),
            segments,
        })
    }

    fn identity(source: &str) -> Self {
        let range = TextRange::new(0, source.len());
        Self {
            original_len: source.len(),
            virtual_len: source.len(),
            segments: vec![SourceMapSegment::new(
                range,
                SourceOrigin::exact(range, None),
            )],
        }
    }

    fn validate_segments(
        original: &str,
        virtual_source: &str,
        segments: &[SourceMapSegment],
        expansions: &ExpansionGraph,
    ) -> Result<(), SourceMapError> {
        if virtual_source.is_empty() {
            if segments.len() != 1 || segments[0].virtual_range != TextRange::empty(0) {
                return Err(SourceMapError::EmptyVirtualSourceMapping);
            }
            return Self::validate_segment(original, virtual_source, segments[0], expansions);
        }

        let mut expected_start = 0;
        for segment in segments {
            if segment.virtual_range.is_empty() {
                return Err(SourceMapError::EmptySegment {
                    range: segment.virtual_range,
                });
            }
            if segment.virtual_range.start < expected_start {
                return Err(SourceMapError::Overlap {
                    previous_end: expected_start,
                    next_start: segment.virtual_range.start,
                });
            }
            if segment.virtual_range.start > expected_start {
                return Err(SourceMapError::Gap {
                    expected_start,
                    actual_start: segment.virtual_range.start,
                });
            }
            Self::validate_segment(original, virtual_source, *segment, expansions)?;
            expected_start = segment.virtual_range.end;
        }
        if expected_start != virtual_source.len() {
            return Err(SourceMapError::IncompleteCoverage {
                covered_end: expected_start,
                virtual_len: virtual_source.len(),
            });
        }
        Ok(())
    }

    fn validate_segment(
        original: &str,
        virtual_source: &str,
        segment: SourceMapSegment,
        expansions: &ExpansionGraph,
    ) -> Result<(), SourceMapError> {
        if !segment.virtual_range.is_valid_for(virtual_source) {
            return Err(SourceMapError::InvalidRange {
                input: "virtual source",
                range: segment.virtual_range,
            });
        }
        if !segment.origin.original_range.is_valid_for(original) {
            return Err(SourceMapError::InvalidRange {
                input: "original source",
                range: segment.origin.original_range,
            });
        }
        if let Some(expansion) = segment.origin.expansion
            && !expansions.contains(expansion)
        {
            return Err(SourceMapError::UnknownExpansion { expansion });
        }

        match segment.origin.kind {
            OriginKind::Exact => {
                if segment.virtual_range.len() != segment.origin.original_range.len() {
                    return Err(SourceMapError::ExactLengthMismatch {
                        virtual_range: segment.virtual_range,
                        original_range: segment.origin.original_range,
                    });
                }
                if segment.virtual_range.slice(virtual_source)
                    != segment.origin.original_range.slice(original)
                {
                    return Err(SourceMapError::ExactTextMismatch {
                        virtual_range: segment.virtual_range,
                        original_range: segment.origin.original_range,
                    });
                }
            }
            OriginKind::Replaced => {}
            OriginKind::Anchored => {
                if !segment.origin.original_range.is_empty() {
                    return Err(SourceMapError::NonEmptyAnchor {
                        range: segment.origin.original_range,
                    });
                }
                if segment.origin.expansion.is_none() {
                    return Err(SourceMapError::AnchorWithoutExpansion);
                }
            }
        }
        Ok(())
    }

    fn map_range(&self, range: TextRange) -> Result<MappedSpan, SourceMapError> {
        if range.start > range.end || range.end > self.virtual_len {
            return Err(SourceMapError::InvalidRange {
                input: "virtual source",
                range,
            });
        }

        let origins = if range.is_empty() {
            vec![self.map_point(range.start)]
        } else {
            let mut origins = Vec::new();
            for segment in &self.segments {
                let Some(overlap) = segment.virtual_range.intersection(range) else {
                    continue;
                };
                let mapped = Self::map_overlap(*segment, overlap);
                if origins.last() != Some(&mapped) {
                    origins.push(mapped);
                }
            }
            origins
        };

        Ok(MappedSpan {
            virtual_range: range,
            origins,
        })
    }

    fn map_point(&self, offset: usize) -> SourceOrigin {
        let segment = if self.virtual_len == 0 {
            &self.segments[0]
        } else if offset == self.virtual_len {
            self.segments
                .last()
                .expect("validated source map has segments")
        } else {
            self.segments
                .iter()
                .find(|segment| {
                    segment.virtual_range.start <= offset && offset < segment.virtual_range.end
                })
                .expect("validated source map covers virtual source")
        };

        match segment.origin.kind {
            OriginKind::Exact => {
                let delta = offset.saturating_sub(segment.virtual_range.start);
                SourceOrigin::exact(
                    TextRange::empty(segment.origin.original_range.start + delta),
                    segment.origin.expansion,
                )
            }
            OriginKind::Replaced | OriginKind::Anchored => segment.origin,
        }
    }

    fn map_overlap(segment: SourceMapSegment, overlap: TextRange) -> SourceOrigin {
        match segment.origin.kind {
            OriginKind::Exact => {
                let start_delta = overlap.start - segment.virtual_range.start;
                let end_delta = overlap.end - segment.virtual_range.start;
                SourceOrigin::exact(
                    TextRange::new(
                        segment.origin.original_range.start + start_delta,
                        segment.origin.original_range.start + end_delta,
                    ),
                    segment.origin.expansion,
                )
            }
            OriginKind::Replaced | OriginKind::Anchored => segment.origin,
        }
    }

    pub fn original_len(&self) -> usize {
        self.original_len
    }

    pub fn virtual_len(&self) -> usize {
        self.virtual_len
    }

    pub fn segments(&self) -> &[SourceMapSegment] {
        &self.segments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSpan {
    pub virtual_range: TextRange,
    pub origins: Vec<SourceOrigin>,
}

impl MappedSpan {
    pub fn primary_origin(&self) -> Option<SourceOrigin> {
        self.origins.first().copied()
    }

    pub fn is_generated(&self) -> bool {
        self.origins
            .iter()
            .any(|origin| origin.expansion.is_some() || origin.kind != OriginKind::Exact)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub replacement: String,
    pub anchor: Option<usize>,
}

impl TextEdit {
    pub fn new(range: TextRange, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
            anchor: None,
        }
    }

    pub fn anchored(range: TextRange, replacement: impl Into<String>, anchor: usize) -> Self {
        Self {
            range,
            replacement: replacement.into(),
            anchor: Some(anchor),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextExpansion {
    pub component: ComponentId,
    pub hook: HookId,
    pub definition_site: Option<ExpansionSite>,
}

impl TextExpansion {
    pub fn new(component: impl Into<String>, hook: impl Into<String>) -> Self {
        Self {
            component: ComponentId::new(component),
            hook: HookId::new(hook),
            definition_site: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditApplication {
    pub source: MappedSource,
    pub expansion: Option<ExpansionId>,
    pub generated_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSource {
    original: Arc<str>,
    virtual_source: Arc<str>,
    source_map: SourceMap,
    expansions: ExpansionGraph,
}

impl MappedSource {
    pub fn identity(source: impl Into<Arc<str>>) -> Self {
        let source = source.into();
        Self {
            source_map: SourceMap::identity(&source),
            original: Arc::clone(&source),
            virtual_source: source,
            expansions: ExpansionGraph::default(),
        }
    }

    pub fn new(
        original: impl Into<Arc<str>>,
        virtual_source: impl Into<Arc<str>>,
        expansions: ExpansionGraph,
        segments: Vec<SourceMapSegment>,
    ) -> Result<Self, SourceMapError> {
        let original = original.into();
        let virtual_source = virtual_source.into();
        for expansion in expansions.iter() {
            Self::validate_expansion_site(
                &original,
                expansion.id,
                "call site",
                expansion.call_site.original_range,
            )?;
            if let Some(definition_site) = expansion.definition_site {
                Self::validate_expansion_site(
                    &original,
                    expansion.id,
                    "definition site",
                    definition_site.original_range,
                )?;
            }
        }
        let source_map = SourceMap::new(&original, &virtual_source, segments, &expansions)?;
        Ok(Self {
            original,
            virtual_source,
            source_map,
            expansions,
        })
    }

    fn validate_expansion_site(
        original: &str,
        expansion: ExpansionId,
        site: &'static str,
        range: TextRange,
    ) -> Result<(), SourceMapError> {
        if !range.is_valid_for(original) {
            return Err(SourceMapError::InvalidExpansionSite {
                expansion,
                site,
                range,
            });
        }
        Ok(())
    }

    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn virtual_source(&self) -> &str {
        &self.virtual_source
    }

    pub fn source_map(&self) -> &SourceMap {
        &self.source_map
    }

    pub fn expansions(&self) -> &ExpansionGraph {
        &self.expansions
    }

    pub fn map_range(&self, range: TextRange) -> Result<MappedSpan, SourceMapError> {
        if !range.is_valid_for(&self.virtual_source) {
            return Err(SourceMapError::InvalidRange {
                input: "virtual source",
                range,
            });
        }
        self.source_map.map_range(range)
    }

    pub fn expansion_backtrace(&self, id: ExpansionId) -> Option<Vec<&crate::Expansion>> {
        self.expansions.backtrace(id)
    }

    pub fn apply_text_edits(
        &self,
        edits: impl IntoIterator<Item = TextEdit>,
        metadata: TextExpansion,
    ) -> Result<TextEditApplication, TextEditError> {
        let mut edits = edits.into_iter().collect::<Vec<_>>();
        if edits.is_empty() {
            return Ok(TextEditApplication {
                source: self.clone(),
                expansion: None,
                generated_bytes: 0,
            });
        }

        edits.sort_by(|left, right| {
            left.range
                .start
                .cmp(&right.range.start)
                .then_with(|| left.range.end.cmp(&right.range.end))
        });
        self.validate_text_edits(&edits)?;

        let first = &edits[0];
        let call_site_span = if let Some(anchor) = first.anchor {
            self.map_range(TextRange::empty(anchor))?
        } else {
            self.map_range(first.range)?
        };
        let call_site_origin = call_site_span
            .primary_origin()
            .expect("a validated source map always returns an origin");
        let expansion_id = self.expansions.next_id()?;
        let expansion = Expansion {
            id: expansion_id,
            kind: ExpansionKind::Text,
            component: metadata.component,
            hook: metadata.hook,
            call_site: ExpansionSite {
                original_range: call_site_origin.original_range,
                expansion: call_site_origin.expansion,
            },
            definition_site: metadata.definition_site,
            syntax_context: SyntaxContextId::new(expansion_id.get()),
        };
        let expansions = self.expansions.with_expansion(expansion)?;

        let generated_bytes = edits.iter().fold(0usize, |total, edit| {
            total.saturating_add(edit.replacement.len())
        });
        let removed_bytes = edits
            .iter()
            .fold(0usize, |total, edit| total.saturating_add(edit.range.len()));
        let mut virtual_source = String::with_capacity(
            self.virtual_source
                .len()
                .saturating_sub(removed_bytes)
                .saturating_add(generated_bytes),
        );
        let mut segments = Vec::new();
        let mut cursor = 0usize;

        for edit in &edits {
            self.append_preserved_range(
                TextRange::new(cursor, edit.range.start),
                &mut virtual_source,
                &mut segments,
            );

            let replacement_start = virtual_source.len();
            virtual_source.push_str(&edit.replacement);
            let replacement_end = virtual_source.len();
            if replacement_start != replacement_end {
                segments.push(SourceMapSegment::new(
                    TextRange::new(replacement_start, replacement_end),
                    self.generated_origin(edit, expansion_id)?,
                ));
            }
            cursor = edit.range.end;
        }
        self.append_preserved_range(
            TextRange::new(cursor, self.virtual_source.len()),
            &mut virtual_source,
            &mut segments,
        );

        if virtual_source.is_empty() {
            segments.push(SourceMapSegment::new(
                TextRange::empty(0),
                self.generated_origin(&edits[0], expansion_id)?,
            ));
        }

        let source = MappedSource::new(
            Arc::clone(&self.original),
            Arc::<str>::from(virtual_source),
            expansions,
            segments,
        )?;
        Ok(TextEditApplication {
            source,
            expansion: Some(expansion_id),
            generated_bytes,
        })
    }

    fn validate_text_edits(&self, edits: &[TextEdit]) -> Result<(), TextEditError> {
        for (index, edit) in edits.iter().enumerate() {
            if !edit.range.is_valid_for(&self.virtual_source) {
                return Err(TextEditError::InvalidRange {
                    index,
                    range: edit.range,
                });
            }
            if edit.range.is_empty() && edit.replacement.is_empty() {
                return Err(TextEditError::NoOpEdit { index });
            }
            if let Some(anchor) = edit.anchor
                && (anchor > self.virtual_source.len()
                    || !self.virtual_source.is_char_boundary(anchor))
            {
                return Err(TextEditError::InvalidAnchor { index, anchor });
            }
        }

        for index in 1..edits.len() {
            let previous = &edits[index - 1];
            let current = &edits[index];
            let overlaps = current.range.start < previous.range.end
                || (current.range.start == previous.range.start
                    && (current.range.is_empty() || previous.range.is_empty()));
            if overlaps {
                return Err(TextEditError::OverlappingEdits {
                    first: index - 1,
                    second: index,
                });
            }
        }
        Ok(())
    }

    fn append_preserved_range(
        &self,
        range: TextRange,
        output: &mut String,
        segments: &mut Vec<SourceMapSegment>,
    ) {
        if range.is_empty() {
            return;
        }
        output.push_str(
            range
                .slice(&self.virtual_source)
                .expect("text edit ranges were validated"),
        );
        let mut output_start = output.len() - range.len();
        for segment in &self.source_map.segments {
            let Some(overlap) = segment.virtual_range.intersection(range) else {
                continue;
            };
            let output_end = output_start + overlap.len();
            segments.push(SourceMapSegment::new(
                TextRange::new(output_start, output_end),
                SourceMap::map_overlap(*segment, overlap),
            ));
            output_start = output_end;
        }
    }

    fn generated_origin(
        &self,
        edit: &TextEdit,
        expansion: ExpansionId,
    ) -> Result<SourceOrigin, SourceMapError> {
        let mapped = if let Some(anchor) = edit.anchor {
            self.map_range(TextRange::empty(anchor))?
        } else {
            self.map_range(edit.range)?
        };
        let origin = mapped
            .primary_origin()
            .expect("a validated source map always returns an origin");
        if origin.original_range.is_empty() {
            Ok(SourceOrigin::anchored(
                origin.original_range.start,
                expansion,
            ))
        } else {
            Ok(SourceOrigin::replaced(
                origin.original_range,
                Some(expansion),
            ))
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TextEditError {
    #[error("text edit {index} has invalid UTF-8 byte range {range}")]
    InvalidRange { index: usize, range: TextRange },
    #[error("text edit {index} has invalid UTF-8 anchor byte {anchor}")]
    InvalidAnchor { index: usize, anchor: usize },
    #[error("text edit {index} does not change the source")]
    NoOpEdit { index: usize },
    #[error("text edits {first} and {second} overlap or have an ambiguous insertion order")]
    OverlappingEdits { first: usize, second: usize },
    #[error(transparent)]
    Expansion(#[from] ExpansionGraphError),
    #[error(transparent)]
    SourceMap(#[from] SourceMapError),
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SourceMapError {
    #[error("invalid {input} byte range {range}")]
    InvalidRange {
        input: &'static str,
        range: TextRange,
    },
    #[error("empty virtual source must have exactly one zero-width mapping at byte 0")]
    EmptyVirtualSourceMapping,
    #[error("non-empty virtual source has an empty mapping segment at {range}")]
    EmptySegment { range: TextRange },
    #[error(
        "source map segments overlap: previous ends at {previous_end}, next starts at {next_start}"
    )]
    Overlap {
        previous_end: usize,
        next_start: usize,
    },
    #[error("source map has a gap: expected byte {expected_start}, next starts at {actual_start}")]
    Gap {
        expected_start: usize,
        actual_start: usize,
    },
    #[error(
        "source map covers through byte {covered_end}, but virtual source length is {virtual_len}"
    )]
    IncompleteCoverage {
        covered_end: usize,
        virtual_len: usize,
    },
    #[error(
        "exact mapping length mismatch between virtual {virtual_range} and original {original_range}"
    )]
    ExactLengthMismatch {
        virtual_range: TextRange,
        original_range: TextRange,
    },
    #[error(
        "exact mapping text mismatch between virtual {virtual_range} and original {original_range}"
    )]
    ExactTextMismatch {
        virtual_range: TextRange,
        original_range: TextRange,
    },
    #[error("anchor mapping must use an empty original range, found {range}")]
    NonEmptyAnchor { range: TextRange },
    #[error("anchor mapping must reference the expansion that generated it")]
    AnchorWithoutExpansion,
    #[error("source map references unknown expansion {expansion}")]
    UnknownExpansion { expansion: ExpansionId },
    #[error("expansion {expansion} has invalid {site} range {range} in the original source")]
    InvalidExpansionSite {
        expansion: ExpansionId,
        site: &'static str,
        range: TextRange,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentId, Expansion, ExpansionKind, ExpansionSite, HookId, SyntaxContextId};
    use proptest::prelude::*;

    fn expansion(id: u32, parent: Option<u32>, range: TextRange) -> Expansion {
        Expansion {
            id: ExpansionId::new(id),
            kind: ExpansionKind::Text,
            component: ComponentId::from("test-macro"),
            hook: HookId::from("expand"),
            call_site: ExpansionSite {
                original_range: range,
                expansion: parent.map(ExpansionId::new),
            },
            definition_site: None,
            syntax_context: SyntaxContextId::new(id),
        }
    }

    #[test]
    fn identity_source_maps_utf8_ranges_exactly() {
        let source = MappedSource::identity("start 日本語 🙂 end");
        let range = TextRange::new(6, 15);
        let mapped = source.map_range(range).unwrap();
        assert_eq!(range.slice(source.virtual_source()), Some("日本語"));
        assert_eq!(
            mapped.primary_origin(),
            Some(SourceOrigin::exact(range, None))
        );
        assert!(!mapped.is_generated());
    }

    #[test]
    fn generated_text_maps_to_anchor_and_keeps_backtrace() {
        let original = "send value";
        let virtual_source = "send generated value";
        let graph = ExpansionGraph::new([
            expansion(1, None, TextRange::new(0, 10)),
            expansion(2, Some(1), TextRange::empty(4)),
        ])
        .unwrap();
        let source = MappedSource::new(
            original,
            virtual_source,
            graph,
            vec![
                SourceMapSegment::new(
                    TextRange::new(0, 4),
                    SourceOrigin::exact(TextRange::new(0, 4), None),
                ),
                SourceMapSegment::new(
                    TextRange::new(4, 14),
                    SourceOrigin::anchored(4, ExpansionId::new(2)),
                ),
                SourceMapSegment::new(
                    TextRange::new(14, 20),
                    SourceOrigin::exact(TextRange::new(4, 10), None),
                ),
            ],
        )
        .unwrap();

        let generated = source.map_range(TextRange::new(5, 14)).unwrap();
        assert_eq!(
            generated.primary_origin(),
            Some(SourceOrigin::anchored(4, ExpansionId::new(2)))
        );
        assert!(generated.is_generated());
        let ids = source
            .expansion_backtrace(ExpansionId::new(2))
            .unwrap()
            .into_iter()
            .map(|item| item.id.get())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![2, 1]);
    }

    #[test]
    fn replacement_maps_partial_virtual_ranges_to_complete_origin() {
        let graph = ExpansionGraph::new([expansion(1, None, TextRange::new(0, 3))]).unwrap();
        let source = MappedSource::new(
            "foo",
            "barbar",
            graph,
            vec![SourceMapSegment::new(
                TextRange::new(0, 6),
                SourceOrigin::replaced(TextRange::new(0, 3), Some(ExpansionId::new(1))),
            )],
        )
        .unwrap();

        assert_eq!(
            source.map_range(TextRange::new(1, 4)).unwrap().origins,
            vec![SourceOrigin::replaced(
                TextRange::new(0, 3),
                Some(ExpansionId::new(1))
            )]
        );
    }

    #[test]
    fn validates_complete_non_overlapping_segments() {
        let gap = MappedSource::new(
            "abc",
            "abc",
            ExpansionGraph::default(),
            vec![
                SourceMapSegment::new(
                    TextRange::new(0, 1),
                    SourceOrigin::exact(TextRange::new(0, 1), None),
                ),
                SourceMapSegment::new(
                    TextRange::new(2, 3),
                    SourceOrigin::exact(TextRange::new(2, 3), None),
                ),
            ],
        )
        .unwrap_err();
        assert!(matches!(gap, SourceMapError::Gap { .. }));

        let mismatch = MappedSource::new(
            "abc",
            "abd",
            ExpansionGraph::default(),
            vec![SourceMapSegment::new(
                TextRange::new(0, 3),
                SourceOrigin::exact(TextRange::new(0, 3), None),
            )],
        )
        .unwrap_err();
        assert!(matches!(mismatch, SourceMapError::ExactTextMismatch { .. }));
    }

    #[test]
    fn validates_expansion_sites_against_original_utf8() {
        let graph = ExpansionGraph::new([expansion(1, None, TextRange::new(1, 2))]).unwrap();
        let error = MappedSource::new(
            "日本語",
            "x",
            graph,
            vec![SourceMapSegment::new(
                TextRange::new(0, 1),
                SourceOrigin::anchored(0, ExpansionId::new(1)),
            )],
        )
        .unwrap_err();
        assert!(matches!(error, SourceMapError::InvalidExpansionSite { .. }));
    }

    #[test]
    fn maps_empty_sources_and_cursors() {
        let empty = MappedSource::identity("");
        assert_eq!(
            empty.map_range(TextRange::empty(0)).unwrap().origins,
            vec![SourceOrigin::exact(TextRange::empty(0), None)]
        );

        let source = MappedSource::identity("abc");
        assert_eq!(
            source.map_range(TextRange::empty(3)).unwrap().origins,
            vec![SourceOrigin::exact(TextRange::empty(3), None)]
        );
    }

    proptest! {
        #[test]
        fn identity_mapping_preserves_all_utf8_ranges(chars in proptest::collection::vec(any::<char>(), 0..20)) {
            let text = chars.into_iter().collect::<String>();
            let source = MappedSource::identity(text.clone());
            let boundaries = text
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(text.len()))
                .collect::<Vec<_>>();

            for &start in &boundaries {
                for &end in boundaries.iter().filter(|&&end| end >= start) {
                    let range = TextRange::new(start, end);
                    prop_assert_eq!(
                        source.map_range(range).unwrap().primary_origin(),
                        Some(SourceOrigin::exact(range, None))
                    );
                }
            }
        }
    }

    #[test]
    fn applies_sorted_edits_and_preserves_unmodified_origins() {
        let source = MappedSource::identity("alpha beta");
        let applied = source
            .apply_text_edits(
                [
                    TextEdit::new(TextRange::new(6, 10), "二"),
                    TextEdit::new(TextRange::new(0, 5), "one"),
                ],
                TextExpansion::new("test.component", "sorted"),
            )
            .unwrap();

        assert_eq!(applied.source.virtual_source(), "one 二");
        assert_eq!(applied.generated_bytes, "one二".len());
        let space = applied
            .source
            .map_range(TextRange::new(3, 4))
            .unwrap()
            .primary_origin();
        assert_eq!(space, Some(SourceOrigin::exact(TextRange::new(5, 6), None)));
        let replacement = applied
            .source
            .map_range(TextRange::new(4, 7))
            .unwrap()
            .primary_origin()
            .unwrap();
        assert_eq!(replacement.original_range, TextRange::new(6, 10));
        assert_eq!(replacement.kind, OriginKind::Replaced);
        assert_eq!(replacement.expansion, applied.expansion);
    }

    #[test]
    fn rejects_overlapping_ambiguous_and_invalid_utf8_edits() {
        let source = MappedSource::identity("日abc");
        let ascii = MappedSource::identity("abcdef");
        assert!(matches!(
            ascii.apply_text_edits(
                [
                    TextEdit::new(TextRange::new(0, 3), "x"),
                    TextEdit::new(TextRange::new(2, 4), "y"),
                ],
                TextExpansion::new("test.component", "overlap"),
            ),
            Err(TextEditError::OverlappingEdits { .. })
        ));
        assert!(matches!(
            source.apply_text_edits(
                [
                    TextEdit::new(TextRange::new(0, 3), "x"),
                    TextEdit::new(TextRange::new(2, 4), "y"),
                ],
                TextExpansion::new("test.component", "overlap"),
            ),
            Err(TextEditError::InvalidRange { index: 1, .. })
        ));
        assert!(matches!(
            source.apply_text_edits(
                [
                    TextEdit::new(TextRange::empty(3), "x"),
                    TextEdit::new(TextRange::empty(3), "y"),
                ],
                TextExpansion::new("test.component", "ambiguous"),
            ),
            Err(TextEditError::OverlappingEdits { .. })
        ));
        assert!(matches!(
            source.apply_text_edits(
                [TextEdit::anchored(TextRange::new(3, 4), "x", 1)],
                TextExpansion::new("test.component", "anchor"),
            ),
            Err(TextEditError::InvalidAnchor { .. })
        ));
    }

    #[test]
    fn preserves_empty_batches_and_maps_full_deletions() {
        let source = MappedSource::identity("delete me");
        let unchanged = source
            .apply_text_edits([], TextExpansion::new("test.component", "empty-batch"))
            .unwrap();
        assert_eq!(unchanged.source, source);
        assert_eq!(unchanged.expansion, None);
        assert_eq!(unchanged.generated_bytes, 0);

        let deleted = source
            .apply_text_edits(
                [TextEdit::new(TextRange::new(0, "delete me".len()), "")],
                TextExpansion::new("test.component", "delete-all"),
            )
            .unwrap();
        let expansion = deleted.expansion.expect("deletion is an expansion");
        assert_eq!(deleted.source.virtual_source(), "");
        assert_eq!(deleted.source.source_map().segments().len(), 1);
        let origin = deleted
            .source
            .map_range(TextRange::empty(0))
            .unwrap()
            .primary_origin()
            .expect("empty result keeps its deletion origin");
        assert_eq!(origin.original_range, TextRange::new(0, "delete me".len()));
        assert_eq!(origin.kind, OriginKind::Replaced);
        assert_eq!(origin.expansion, Some(expansion));
    }

    #[test]
    fn chained_edits_keep_the_expansion_backtrace() {
        let first = MappedSource::identity("alpha")
            .apply_text_edits(
                [TextEdit::new(TextRange::new(0, 5), "日本")],
                TextExpansion::new("first.component", "first"),
            )
            .unwrap();
        let first_id = first.expansion.unwrap();
        let second = first
            .source
            .apply_text_edits(
                [TextEdit::new(TextRange::new(0, "日本".len()), "done")],
                TextExpansion::new("second.component", "second"),
            )
            .unwrap();
        let second_id = second.expansion.unwrap();

        assert_eq!(second.source.virtual_source(), "done");
        assert_eq!(
            second
                .source
                .expansion_backtrace(second_id)
                .unwrap()
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [second_id, first_id]
        );
        assert_eq!(
            second
                .source
                .map_range(TextRange::new(0, 4))
                .unwrap()
                .primary_origin()
                .unwrap()
                .original_range,
            TextRange::new(0, 5)
        );
    }

    #[test]
    fn explicit_anchor_controls_generated_insertions() {
        let source = MappedSource::identity("left right");
        let applied = source
            .apply_text_edits(
                [TextEdit::anchored(TextRange::empty(0), "generated ", 5)],
                TextExpansion::new("test.component", "anchor"),
            )
            .unwrap();
        let generated = applied
            .source
            .map_range(TextRange::new(0, 9))
            .unwrap()
            .primary_origin()
            .unwrap();
        assert_eq!(generated.original_range, TextRange::empty(5));
        assert_eq!(generated.kind, OriginKind::Anchored);
    }

    proptest! {
        #[test]
        fn arbitrary_utf8_replacements_produce_valid_source_maps(
            chars in proptest::collection::vec(any::<char>(), 1..40),
            replacement_chars in proptest::collection::vec(any::<char>(), 0..20),
            first in any::<usize>(),
            second in any::<usize>(),
        ) {
            let source = chars.into_iter().collect::<String>();
            let replacement = replacement_chars.into_iter().collect::<String>();
            let boundaries = source
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(source.len()))
                .collect::<Vec<_>>();
            let left = first % boundaries.len();
            let right = second % boundaries.len();
            let start = boundaries[left.min(right)];
            let end = boundaries[left.max(right)];
            if start == end && replacement.is_empty() {
                return Ok(());
            }

            let applied = MappedSource::identity(source.clone())
                .apply_text_edits(
                    [TextEdit::new(TextRange::new(start, end), replacement)],
                    TextExpansion::new("proptest.component", "replace"),
                )
                .unwrap();
            for segment in applied.source.source_map().segments() {
                prop_assert!(segment.virtual_range.is_valid_for(applied.source.virtual_source()));
                prop_assert!(segment.origin.original_range.is_valid_for(&source));
            }
        }
    }
}
