//! Mapping between original document bytes and macro-expanded virtual source.
//!
//! Text edits are validated and applied atomically. Generated segments retain all
//! relevant origins and expansion IDs so diagnostics can return to editor source.
#![allow(missing_docs)] // Type-level docs describe aggregate field contracts.

use crate::{
    ComponentId, Expansion, ExpansionGraph, ExpansionGraphError, ExpansionId, ExpansionKind,
    ExpansionSite, HookId, SyntaxContextId, TextRange,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// How a virtual source segment relates to its original range.
pub enum OriginKind {
    /// The virtual text is byte-for-byte identical to the original text.
    Exact,
    /// The virtual text replaces the complete original range.
    Replaced,
    /// The virtual text was generated at a zero-width original position.
    Anchored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// One original-document origin associated with a virtual segment.
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Contiguous virtual range and all original origins contributing to it.
pub struct SourceMapSegment {
    pub virtual_range: TextRange,
    pub origins: Vec<SourceOrigin>,
}

impl SourceMapSegment {
    /// Creates a segment with one original-source origin.
    pub fn new(virtual_range: TextRange, origin: SourceOrigin) -> Self {
        Self {
            virtual_range,
            origins: vec![origin],
        }
    }

    pub fn with_origins(
        virtual_range: TextRange,
        origins: impl IntoIterator<Item = SourceOrigin>,
    ) -> Self {
        Self {
            virtual_range,
            origins: origins.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Validated ordered mapping from virtual UTF-8 ranges to original source.
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
            return Self::validate_segment(original, virtual_source, &segments[0], expansions);
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
            Self::validate_segment(original, virtual_source, segment, expansions)?;
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
        segment: &SourceMapSegment,
        expansions: &ExpansionGraph,
    ) -> Result<(), SourceMapError> {
        if !segment.virtual_range.is_valid_for(virtual_source) {
            return Err(SourceMapError::InvalidRange {
                input: "virtual source",
                range: segment.virtual_range,
            });
        }
        if segment.origins.is_empty() {
            return Err(SourceMapError::MissingOrigins {
                range: segment.virtual_range,
            });
        }
        if segment.origins.len() > 1
            && segment
                .origins
                .iter()
                .any(|origin| matches!(origin.kind, OriginKind::Exact))
        {
            return Err(SourceMapError::MultipleExactOrigins {
                range: segment.virtual_range,
            });
        }

        for origin in &segment.origins {
            if !origin.original_range.is_valid_for(original) {
                return Err(SourceMapError::InvalidRange {
                    input: "original source",
                    range: origin.original_range,
                });
            }
            if let Some(expansion) = origin.expansion
                && !expansions.contains(expansion)
            {
                return Err(SourceMapError::UnknownExpansion { expansion });
            }

            match origin.kind {
                OriginKind::Exact => {
                    if segment.virtual_range.len() != origin.original_range.len() {
                        return Err(SourceMapError::ExactLengthMismatch {
                            virtual_range: segment.virtual_range,
                            original_range: origin.original_range,
                        });
                    }
                    if segment.virtual_range.slice(virtual_source)
                        != origin.original_range.slice(original)
                    {
                        return Err(SourceMapError::ExactTextMismatch {
                            virtual_range: segment.virtual_range,
                            original_range: origin.original_range,
                        });
                    }
                }
                OriginKind::Replaced => {}
                OriginKind::Anchored => {
                    if !origin.original_range.is_empty() {
                        return Err(SourceMapError::NonEmptyAnchor {
                            range: origin.original_range,
                        });
                    }
                    if origin.expansion.is_none() {
                        return Err(SourceMapError::AnchorWithoutExpansion);
                    }
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
            self.map_point(range.start)
        } else {
            let mut origins = Vec::new();
            for segment in &self.segments {
                let Some(overlap) = segment.virtual_range.intersection(range) else {
                    continue;
                };
                for mapped in Self::map_overlap(segment, overlap) {
                    if !origins.contains(&mapped) {
                        origins.push(mapped);
                    }
                }
            }
            origins
        };

        Ok(MappedSpan {
            virtual_range: range,
            origins,
        })
    }

    fn map_point(&self, offset: usize) -> Vec<SourceOrigin> {
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

        segment
            .origins
            .iter()
            .map(|origin| match origin.kind {
                OriginKind::Exact => {
                    let delta = offset.saturating_sub(segment.virtual_range.start);
                    SourceOrigin::exact(
                        TextRange::empty(origin.original_range.start + delta),
                        origin.expansion,
                    )
                }
                OriginKind::Replaced | OriginKind::Anchored => *origin,
            })
            .collect()
    }

    fn map_overlap(segment: &SourceMapSegment, overlap: TextRange) -> Vec<SourceOrigin> {
        segment
            .origins
            .iter()
            .map(|origin| match origin.kind {
                OriginKind::Exact => {
                    let start_delta = overlap.start - segment.virtual_range.start;
                    let end_delta = overlap.end - segment.virtual_range.start;
                    SourceOrigin::exact(
                        TextRange::new(
                            origin.original_range.start + start_delta,
                            origin.original_range.start + end_delta,
                        ),
                        origin.expansion,
                    )
                }
                OriginKind::Replaced | OriginKind::Anchored => *origin,
            })
            .collect()
    }

    /// Returns the original document length in UTF-8 bytes.
    pub fn original_len(&self) -> usize {
        self.original_len
    }

    /// Returns the current virtual source length in UTF-8 bytes.
    pub fn virtual_len(&self) -> usize {
        self.virtual_len
    }

    /// Returns validated segments in contiguous virtual order.
    pub fn segments(&self) -> &[SourceMapSegment] {
        &self.segments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Local virtual range plus every editor-facing original origin.
pub struct MappedSpan {
    pub virtual_range: TextRange,
    pub origins: Vec<SourceOrigin>,
}

impl MappedSpan {
    /// Returns the first origin for consumers that cannot display related locations.
    pub fn primary_origin(&self) -> Option<SourceOrigin> {
        self.origins.first().copied()
    }

    /// Returns whether any origin came from a macro expansion.
    pub fn is_generated(&self) -> bool {
        self.origins
            .iter()
            .any(|origin| origin.expansion.is_some() || origin.kind != OriginKind::Exact)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Atomic replacement requested by one Text macro invocation.
pub struct TextEdit {
    pub range: TextRange,
    pub replacement: String,
    pub anchor: Option<usize>,
}

impl TextEdit {
    /// Creates an unanchored replacement edit.
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
/// Identity metadata attached to an accepted Text macro batch.
pub struct TextExpansion {
    pub component: ComponentId,
    pub hook: HookId,
    pub definition_site: Option<ExpansionSite>,
}

impl TextExpansion {
    /// Creates expansion metadata from the owning component and hook.
    pub fn new(component: impl Into<String>, hook: impl Into<String>) -> Self {
        Self {
            component: ComponentId::new(component),
            hook: HookId::new(hook),
            definition_site: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Identity metadata attached to an accepted Tree macro edit.
pub struct TreeExpansion {
    pub component: ComponentId,
    pub hook: HookId,
    pub definition_site: Option<ExpansionSite>,
}

impl TreeExpansion {
    /// Creates expansion metadata from the owning component and hook.
    pub fn new(component: impl Into<String>, hook: impl Into<String>) -> Self {
        Self {
            component: ComponentId::new(component),
            hook: HookId::new(hook),
            definition_site: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// New source provenance and assigned identity after a Tree edit.
pub struct TreeExpansionApplication {
    pub source: MappedSource,
    pub expansion: ExpansionId,
    pub syntax_context: SyntaxContextId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Mapped source and accounting returned after an accepted text-edit batch.
pub struct TextEditApplication {
    pub source: MappedSource,
    pub expansion: Option<ExpansionId>,
    pub generated_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Original text, current virtual text, composed source map, and expansion graph.
///
/// Text macros operate on the virtual source while editor-facing diagnostics
/// must point into the immutable original document. This type keeps those two
/// views together and composes provenance across successive macro expansions.
///
/// # Examples
///
/// Replacing text records both the macro identity and the original source range:
///
/// ~~~
/// use skript_parser::{
///     MappedSource, OriginKind, TextEdit, TextExpansion, TextRange,
/// };
///
/// let source = MappedSource::identity("print value");
/// let applied = source.apply_text_edits(
///     [TextEdit::new(TextRange::new(6, 11), "42")],
///     TextExpansion::new("example.addon", "replace-value"),
/// )?;
///
/// assert_eq!(applied.source.original(), "print value");
/// assert_eq!(applied.source.virtual_source(), "print 42");
///
/// let generated = applied.source.map_range(TextRange::new(6, 8))?;
/// let origin = generated.primary_origin().expect("generated text has an origin");
/// assert_eq!(origin.original_range, TextRange::new(6, 11));
/// assert_eq!(origin.kind, OriginKind::Replaced);
/// assert_eq!(origin.expansion, applied.expansion);
/// assert!(generated.is_generated());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ~~~
pub struct MappedSource {
    original: Arc<str>,
    virtual_source: Arc<str>,
    source_map: SourceMap,
    expansions: ExpansionGraph,
}

impl MappedSource {
    /// Creates an unexpanded source whose virtual and original text are identical.
    pub fn identity(source: impl Into<Arc<str>>) -> Self {
        let source = source.into();
        Self {
            source_map: SourceMap::identity(&source),
            original: Arc::clone(&source),
            virtual_source: source,
            expansions: ExpansionGraph::default(),
        }
    }

    /// Constructs a mapped source after validating graph sites and complete segment coverage.
    pub fn new(
        original: impl Into<Arc<str>>,
        virtual_source: impl Into<Arc<str>>,
        expansions: ExpansionGraph,
        segments: Vec<SourceMapSegment>,
    ) -> Result<Self, SourceMapError> {
        let original = original.into();
        let virtual_source = virtual_source.into();
        for expansion in expansions.iter() {
            for call_site in &expansion.call_sites {
                Self::validate_expansion_site(
                    &original,
                    expansion.id,
                    "call site",
                    call_site.original_range,
                )?;
            }
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

    /// Returns the immutable editor document text.
    pub fn original(&self) -> &str {
        &self.original
    }

    /// Returns text after every accepted Text macro transformation.
    pub fn virtual_source(&self) -> &str {
        &self.virtual_source
    }

    /// Returns the composed mapping from virtual to original bytes.
    pub fn source_map(&self) -> &SourceMap {
        &self.source_map
    }

    /// Returns accepted Text, Tree, and AST expansion provenance.
    pub fn expansions(&self) -> &ExpansionGraph {
        &self.expansions
    }

    /// Maps one valid virtual range to all contributing original locations.
    pub fn map_range(&self, range: TextRange) -> Result<MappedSpan, SourceMapError> {
        if !range.is_valid_for(&self.virtual_source) {
            return Err(SourceMapError::InvalidRange {
                input: "virtual source",
                range,
            });
        }
        self.source_map.map_range(range)
    }

    /// Returns the primary expansion path from `id` to original source.
    pub fn expansion_backtrace(&self, id: ExpansionId) -> Option<Vec<&crate::Expansion>> {
        self.expansions.backtrace(id)
    }

    /// Returns every distinct expansion path from `id` to original source.
    pub fn expansion_backtraces(&self, id: ExpansionId) -> Option<Vec<Vec<&crate::Expansion>>> {
        self.expansions.backtraces(id)
    }

    /// Registers accepted Tree provenance without changing source text.
    pub fn register_tree_expansion(
        &self,
        call_site: &MappedSpan,
        metadata: TreeExpansion,
    ) -> Result<TreeExpansionApplication, TreeExpansionError> {
        if !call_site.virtual_range.is_valid_for(&self.virtual_source) {
            return Err(TreeExpansionError::InvalidVirtualRange {
                range: call_site.virtual_range,
            });
        }
        if call_site.origins.is_empty() {
            return Err(TreeExpansionError::MissingOrigins);
        }

        let mut call_sites = Vec::new();
        for origin in &call_site.origins {
            if !origin.original_range.is_valid_for(&self.original) {
                return Err(TreeExpansionError::InvalidOriginalRange {
                    range: origin.original_range,
                });
            }
            if let Some(expansion) = origin.expansion
                && !self.expansions.contains(expansion)
            {
                return Err(TreeExpansionError::UnknownExpansion { expansion });
            }
            let site = ExpansionSite {
                original_range: origin.original_range,
                expansion: origin.expansion,
            };
            if !call_sites.contains(&site) {
                call_sites.push(site);
            }
        }

        let expansion_id = self.expansions.next_id()?;
        let syntax_context = SyntaxContextId::new(expansion_id.get());
        let expansion = Expansion {
            id: expansion_id,
            kind: ExpansionKind::Tree,
            component: metadata.component,
            hook: metadata.hook,
            call_sites,
            definition_site: metadata.definition_site,
            syntax_context,
        };
        let mut source = self.clone();
        source.expansions = self.expansions.with_expansion(expansion)?;
        Ok(TreeExpansionApplication {
            source,
            expansion: expansion_id,
            syntax_context,
        })
    }

    /// Validates and atomically applies one Text macro edit batch.
    ///
    /// Edits may be supplied in any order, but their ranges must not overlap.
    /// Empty batches return an unchanged clone without allocating an expansion
    /// ID. Inserted text can use [TextEdit::anchored] when its diagnostic origin
    /// should be a specific zero-width call site.
    ///
    /// # Errors
    ///
    /// Returns [TextEditError] when a range is invalid, edits overlap, an anchor
    /// is outside the replaced range, or the composed source map would violate
    /// its coverage and provenance invariants. No partial edit is applied.
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

        let expansion_id = self.expansions.next_id()?;
        let expansion = Expansion {
            id: expansion_id,
            kind: ExpansionKind::Text,
            component: metadata.component,
            hook: metadata.hook,
            call_sites: self.text_edit_call_sites(&edits)?,
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
                segments.push(SourceMapSegment::with_origins(
                    TextRange::new(replacement_start, replacement_end),
                    self.generated_origins(edit, expansion_id)?,
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
            segments.push(SourceMapSegment::with_origins(
                TextRange::empty(0),
                self.generated_origins_for_edits(&edits, expansion_id)?,
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

    fn mapped_edit_span(&self, edit: &TextEdit) -> Result<MappedSpan, SourceMapError> {
        if let Some(anchor) = edit.anchor {
            self.map_range(TextRange::empty(anchor))
        } else {
            self.map_range(edit.range)
        }
    }

    fn text_edit_call_sites(
        &self,
        edits: &[TextEdit],
    ) -> Result<Vec<ExpansionSite>, SourceMapError> {
        let mut call_sites = Vec::new();
        for edit in edits {
            for origin in self.mapped_edit_span(edit)?.origins {
                let call_site = ExpansionSite {
                    original_range: origin.original_range,
                    expansion: origin.expansion,
                };
                if !call_sites.contains(&call_site) {
                    call_sites.push(call_site);
                }
            }
        }
        Ok(call_sites)
    }

    fn generated_origins_for_edits(
        &self,
        edits: &[TextEdit],
        expansion: ExpansionId,
    ) -> Result<Vec<SourceOrigin>, SourceMapError> {
        let mut origins = Vec::new();
        for edit in edits {
            for origin in self.generated_origins(edit, expansion)? {
                if !origins.contains(&origin) {
                    origins.push(origin);
                }
            }
        }
        Ok(origins)
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
            segments.push(SourceMapSegment::with_origins(
                TextRange::new(output_start, output_end),
                SourceMap::map_overlap(segment, overlap),
            ));
            output_start = output_end;
        }
    }

    fn generated_origins(
        &self,
        edit: &TextEdit,
        expansion: ExpansionId,
    ) -> Result<Vec<SourceOrigin>, SourceMapError> {
        let mut origins = Vec::new();
        for origin in self.mapped_edit_span(edit)?.origins {
            let generated = if origin.original_range.is_empty() {
                SourceOrigin::anchored(origin.original_range.start, expansion)
            } else {
                SourceOrigin::replaced(origin.original_range, Some(expansion))
            };
            if !origins.contains(&generated) {
                origins.push(generated);
            }
        }
        Ok(origins)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Invalid attempt to register Tree expansion provenance.
pub enum TreeExpansionError {
    #[error("tree macro call site has invalid virtual range {range}")]
    InvalidVirtualRange { range: TextRange },
    #[error("tree macro call site has no source origins")]
    MissingOrigins,
    #[error("tree macro call site has invalid original range {range}")]
    InvalidOriginalRange { range: TextRange },
    #[error("tree macro call site references unknown expansion {expansion}")]
    UnknownExpansion { expansion: ExpansionId },
    #[error(transparent)]
    Expansion(#[from] ExpansionGraphError),
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Invalid or unrepresentable Text macro edit batch.
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
/// Invalid source-map structure or range lookup.
pub enum SourceMapError {
    #[error("invalid {input} byte range {range}")]
    InvalidRange {
        input: &'static str,
        range: TextRange,
    },
    #[error("source map segment {range} has no origins")]
    MissingOrigins { range: TextRange },
    #[error("exact source map segment {range} must have exactly one origin")]
    MultipleExactOrigins { range: TextRange },
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
            call_sites: vec![ExpansionSite {
                original_range: range,
                expansion: parent.map(ExpansionId::new),
            }],
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

        let missing = MappedSource::new(
            "a",
            "x",
            ExpansionGraph::default(),
            vec![SourceMapSegment::with_origins(TextRange::new(0, 1), [])],
        )
        .unwrap_err();
        assert_eq!(
            missing,
            SourceMapError::MissingOrigins {
                range: TextRange::new(0, 1)
            }
        );

        let multiple_exact = MappedSource::new(
            "a",
            "a",
            ExpansionGraph::default(),
            vec![SourceMapSegment::with_origins(
                TextRange::new(0, 1),
                [
                    SourceOrigin::exact(TextRange::new(0, 1), None),
                    SourceOrigin::exact(TextRange::new(0, 1), None),
                ],
            )],
        )
        .unwrap_err();
        assert!(matches!(
            multiple_exact,
            SourceMapError::MultipleExactOrigins { .. }
        ));
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

        let split_deletion = MappedSource::identity("ab")
            .apply_text_edits(
                [
                    TextEdit::new(TextRange::new(0, 1), ""),
                    TextEdit::new(TextRange::new(1, 2), ""),
                ],
                TextExpansion::new("test.component", "split-delete-all"),
            )
            .unwrap();
        let expansion = split_deletion.expansion.unwrap();
        assert_eq!(split_deletion.source.virtual_source(), "");
        assert_eq!(
            split_deletion
                .source
                .map_range(TextRange::empty(0))
                .unwrap()
                .origins,
            [
                SourceOrigin::replaced(TextRange::new(0, 1), Some(expansion)),
                SourceOrigin::replaced(TextRange::new(1, 2), Some(expansion)),
            ]
        );
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
    fn chained_multi_edit_expansions_retain_every_origin_and_parent_path() {
        let first = MappedSource::identity("abc")
            .apply_text_edits(
                [
                    TextEdit::new(TextRange::new(0, 1), "A"),
                    TextEdit::new(TextRange::new(2, 3), "C"),
                ],
                TextExpansion::new("first.component", "outer-characters"),
            )
            .unwrap();
        let first_id = first.expansion.unwrap();
        assert_eq!(first.source.virtual_source(), "AbC");
        assert_eq!(
            first.source.expansions().get(first_id).unwrap().call_sites,
            [
                ExpansionSite::original(TextRange::new(0, 1)),
                ExpansionSite::original(TextRange::new(2, 3)),
            ]
        );

        let second = first
            .source
            .apply_text_edits(
                [TextEdit::new(TextRange::new(0, 3), "X")],
                TextExpansion::new("second.component", "merge"),
            )
            .unwrap();
        let second_id = second.expansion.unwrap();
        assert_eq!(second.source.virtual_source(), "X");
        assert_eq!(
            second
                .source
                .map_range(TextRange::new(0, 1))
                .unwrap()
                .origins,
            [
                SourceOrigin::replaced(TextRange::new(0, 1), Some(second_id)),
                SourceOrigin::replaced(TextRange::new(1, 2), Some(second_id)),
                SourceOrigin::replaced(TextRange::new(2, 3), Some(second_id)),
            ]
        );
        assert_eq!(
            second
                .source
                .expansions()
                .get(second_id)
                .unwrap()
                .call_sites,
            [
                ExpansionSite::expanded(TextRange::new(0, 1), first_id),
                ExpansionSite::original(TextRange::new(1, 2)),
                ExpansionSite::expanded(TextRange::new(2, 3), first_id),
            ]
        );
        assert_eq!(
            second
                .source
                .expansion_backtraces(second_id)
                .unwrap()
                .into_iter()
                .map(|path| path.into_iter().map(|item| item.id).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [vec![second_id, first_id], vec![second_id]]
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
                for origin in &segment.origins {
                    prop_assert!(origin.original_range.is_valid_for(&source));
                }
            }
        }
    }
}
