use crate::nlaocs::skript_parser_addon::types::{MappedSpan, OriginKind, SourceOrigin, TextRange};

/// A flattened origin list has no per-origin virtual offsets. Only a single
/// equal-length Exact origin proves an affine child mapping. Keep ambiguous
/// call sites whole rather than manufacture positions inside replaced UTF-8.
pub(crate) fn subspan(parent: &MappedSpan, range: TextRange) -> MappedSpan {
    let whole = range.start == parent.virtual_range.start && range.end == parent.virtual_range.end;
    let affine = parent.origins.len() == 1
        && parent.origins[0].kind == OriginKind::Exact
        && parent.origins[0]
            .original_range
            .end
            .checked_sub(parent.origins[0].original_range.start)
            == parent
                .virtual_range
                .end
                .checked_sub(parent.virtual_range.start)
        && parent.virtual_range.start <= range.start
        && range.start <= range.end
        && range.end <= parent.virtual_range.end;
    MappedSpan {
        virtual_range: range,
        origins: parent
            .origins
            .iter()
            .map(|origin| SourceOrigin {
                original_range: if affine {
                    TextRange {
                        start: origin.original_range.start
                            + (range.start - parent.virtual_range.start),
                        end: origin.original_range.start + (range.end - parent.virtual_range.start),
                    }
                } else {
                    origin.original_range
                },
                kind: if !affine && !whole && origin.kind == OriginKind::Exact {
                    OriginKind::Replaced
                } else {
                    origin.kind
                },
                expansion: origin.expansion,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent(kind: OriginKind, end: u64) -> MappedSpan {
        MappedSpan {
            virtual_range: TextRange {
                start: 100,
                end: 110,
            },
            origins: vec![SourceOrigin {
                original_range: TextRange { start: 200, end },
                kind,
                expansion: Some(7),
            }],
        }
    }

    #[test]
    fn only_single_equal_length_exact_origins_can_be_subdivided() {
        let mapped = subspan(
            &parent(OriginKind::Exact, 210),
            TextRange {
                start: 102,
                end: 107,
            },
        );
        let origin = &mapped.origins[0];
        assert_eq!(
            (origin.original_range.start, origin.original_range.end),
            (202, 207)
        );
        assert_eq!(origin.kind, OriginKind::Exact);
        assert_eq!(origin.expansion, Some(7));
        for kind in [OriginKind::Replaced, OriginKind::Exact] {
            for end in [205, 210, 215] {
                if kind == OriginKind::Exact && end == 210 {
                    continue;
                }
                let mapped = subspan(
                    &parent(kind, end),
                    TextRange {
                        start: 102,
                        end: 107,
                    },
                );
                let origin = &mapped.origins[0];
                assert_eq!(
                    (origin.original_range.start, origin.original_range.end),
                    (200, end)
                );
                assert_eq!(origin.kind, OriginKind::Replaced);
                assert_eq!(origin.expansion, Some(7));
            }
        }
    }

    #[test]
    fn affine_offsets_do_not_overflow_before_subtracting_the_virtual_base() {
        let mut span = parent(OriginKind::Exact, u64::MAX);
        span.origins[0].original_range.start = u64::MAX - 10;
        let child = subspan(
            &span,
            TextRange {
                start: 102,
                end: 107,
            },
        );
        assert_eq!(
            (
                child.origins[0].original_range.start,
                child.origins[0].original_range.end
            ),
            (u64::MAX - 8, u64::MAX - 3)
        );
    }

    #[test]
    fn anchors_and_mixed_origins_keep_whole_call_sites() {
        let mut span = parent(OriginKind::Anchored, 200);
        let point = subspan(
            &span,
            TextRange {
                start: 105,
                end: 105,
            },
        );
        assert_eq!(point.origins[0].original_range.start, 200);
        assert_eq!(point.origins[0].kind, OriginKind::Anchored);
        span.origins
            .push(parent(OriginKind::Exact, 210).origins.remove(0));
        let child = subspan(
            &span,
            TextRange {
                start: 102,
                end: 107,
            },
        );
        assert_eq!(child.origins[0].kind, OriginKind::Anchored);
        assert_eq!(child.origins[1].kind, OriginKind::Replaced);
        assert_eq!(child.origins[1].original_range.start, 200);
        assert_eq!(child.origins[1].original_range.end, 210);
        let whole = subspan(&span, span.virtual_range);
        assert_eq!(whole.origins[1].kind, OriginKind::Exact);
    }

    #[test]
    fn a_replaced_unicode_prefix_never_creates_an_interior_byte_offset() {
        let original = "\r\nが%arg 4%";
        let span = MappedSpan {
            virtual_range: TextRange {
                start: 100,
                end: 108,
            },
            origins: vec![SourceOrigin {
                original_range: TextRange {
                    start: 2,
                    end: original.len() as u64,
                },
                kind: OriginKind::Replaced,
                expansion: Some(9),
            }],
        };
        let child = subspan(
            &span,
            TextRange {
                start: 102,
                end: 103,
            },
        );
        for origin in child.origins {
            assert!(original.is_char_boundary(origin.original_range.start as usize));
            assert!(original.is_char_boundary(origin.original_range.end as usize));
            assert_eq!(origin.kind, OriginKind::Replaced);
        }
    }
}
