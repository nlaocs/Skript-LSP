use crate::{ExpansionGraph, ExpansionId, TextRange};
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
}
