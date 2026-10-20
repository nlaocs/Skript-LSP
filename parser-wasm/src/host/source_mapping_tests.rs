use super::*;

fn request(kind: WitOriginKind, end: u64, expansion: Option<u64>) -> ParseRequest {
    ParseRequest {
        request_id: 1,
        parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.into(),
        input: "0123456789".into(),
        expected_types: Vec::new(),
        options: Vec::new(),
        span: MappedSpan {
            virtual_range: WitTextRange {
                start: 100,
                end: 110,
            },
            origins: vec![WitSourceOrigin {
                original_range: WitTextRange { start: 200, end },
                kind,
                expansion,
            }],
        },
    }
}

fn child(start: usize, end: usize) -> MatchSpan {
    let source = MappedSource::identity("0123456789");
    MatchSpan {
        // This is deliberately different: nested local frames must not affect mapping.
        local_range: ParserTextRange::new(0, end - start),
        mapped: source.map_range(ParserTextRange::new(start, end)).unwrap(),
    }
}

#[test]
fn additional_children_subdivide_only_proven_exact_origins() {
    let exact = nested_span_to_request(&child(2, 7), &request(WitOriginKind::Exact, 210, None));
    assert_eq!(
        (exact.virtual_range.start, exact.virtual_range.end),
        (102, 107)
    );
    assert_eq!(
        (
            exact.origins[0].original_range.start,
            exact.origins[0].original_range.end
        ),
        (202, 207)
    );
    assert_eq!(exact.origins[0].kind, WitOriginKind::Exact);
    for kind in [WitOriginKind::Replaced, WitOriginKind::Exact] {
        for end in [205, 210, 215] {
            if kind == WitOriginKind::Exact && end == 210 {
                continue;
            }
            let result = nested_span_to_request(&child(2, 7), &request(kind, end, Some(9)));
            assert_eq!(
                (
                    result.origins[0].original_range.start,
                    result.origins[0].original_range.end
                ),
                (200, end)
            );
            assert_eq!(result.origins[0].kind, WitOriginKind::Replaced);
            assert_eq!(result.origins[0].expansion, Some(9));
        }
    }
}

#[test]
fn additional_mixed_origins_do_not_claim_affine_offsets() {
    let mut parent = request(WitOriginKind::Exact, 210, Some(1));
    parent.span.origins.push(
        request(WitOriginKind::Anchored, 200, Some(2))
            .span
            .origins
            .remove(0),
    );
    let result = nested_span_to_request(&child(2, 7), &parent);
    assert_eq!(result.origins.len(), 2);
    assert_eq!(result.origins[0].original_range.start, 200);
    assert_eq!(result.origins[0].original_range.end, 210);
    assert_eq!(result.origins[0].kind, WitOriginKind::Replaced);
    assert_eq!(result.origins[1].kind, WitOriginKind::Anchored);
    assert_eq!(result.origins[1].expansion, Some(2));
    let whole = nested_span_to_request(&child(0, 10), &parent);
    assert_eq!(whole.origins[0].kind, WitOriginKind::Exact);
}

#[test]
fn native_omission_anchors_require_expansion_and_never_invent_exact_positions() {
    let mut absent = child(4, 4);
    absent.mapped.origins[0].kind = ParserOriginKind::Anchored;
    let identity = nested_span_to_request(&absent, &request(WitOriginKind::Exact, 210, None));
    assert_eq!(identity.origins[0].kind, WitOriginKind::Exact);
    assert_eq!(
        (
            identity.origins[0].original_range.start,
            identity.origins[0].original_range.end
        ),
        (204, 204)
    );
    let replaced = nested_span_to_request(&absent, &request(WitOriginKind::Replaced, 215, Some(7)));
    assert_eq!(replaced.origins[0].kind, WitOriginKind::Anchored);
    assert_eq!(
        (
            replaced.origins[0].original_range.start,
            replaced.origins[0].original_range.end
        ),
        (215, 215)
    );
    let ordinary = nested_span_to_request(
        &child(4, 4),
        &request(WitOriginKind::Replaced, 215, Some(7)),
    );
    assert_eq!(ordinary.origins[0].kind, WitOriginKind::Replaced);
    assert_eq!(
        (
            ordinary.origins[0].original_range.start,
            ordinary.origins[0].original_range.end
        ),
        (200, 215)
    );
    let anchored = nested_span_to_request(&absent, &request(WitOriginKind::Anchored, 200, Some(8)));
    assert_eq!(anchored.origins[0].original_range.start, 200);
    assert_eq!(anchored.origins[0].expansion, Some(8));
}
