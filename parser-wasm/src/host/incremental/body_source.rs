//! A local geometry proof, not a replacement for source provenance or authority.
use skript_parser::{MappedSource, OriginKind, SourceMapSegment, SourceOrigin, TextRange};

pub(super) fn original_extent(
    source: &MappedSource,
    range: TextRange,
    witness: impl FnMut(TextRange, SourceOrigin) -> std::fmt::Result,
) -> Result<TextRange, &'static str> {
    prove_extent(source.original(), source.virtual_source(), range,
        source.source_map().segments(), witness)
}

fn prove_extent(
    original: &str,
    current: &str,
    range: TextRange,
    segments: &[SourceMapSegment],
    mut witness: impl FnMut(TextRange, SourceOrigin) -> std::fmt::Result,
) -> Result<TextRange, &'static str> {
    if range.is_empty() || !range.is_valid_for(current) {
        return Err("body_admission.prepare.reject.mapping_range");
    }
    let mut cursor=range.start;
    let mut first=None;
    let mut original_cursor=None;
    for segment in segments {
        let start=segment.virtual_range.start.max(range.start);
        let end=segment.virtual_range.end.min(range.end);
        if start>=end {continue;}
        if start!=cursor || !segment.virtual_range.is_valid_for(current) {
            return Err("body_admission.prepare.reject.mapping_shape");
        }
        let [origin]=segment.origins.as_slice() else {
            return Err("body_admission.prepare.reject.mapping_not_exact");
        };
        if origin.kind!=OriginKind::Exact || origin.expansion.is_some()
            || origin.original_range.len()!=segment.virtual_range.len() {
            return Err("body_admission.prepare.reject.mapping_not_exact");
        }
        let mapped_start=origin.original_range.start.checked_add(start-segment.virtual_range.start)
            .ok_or("body_admission.prepare.reject.mapping_range")?;
        let mapped_end=mapped_start.checked_add(end-start)
            .ok_or("body_admission.prepare.reject.mapping_range")?;
        let mapped=TextRange::new(mapped_start,mapped_end);
        let clipped=TextRange::new(start,end);
        if !origin.original_range.is_valid_for(original) || mapped_end>origin.original_range.end {
            return Err("body_admission.prepare.reject.mapping_range");
        }
        let old=mapped.slice(original).ok_or("body_admission.prepare.reject.mapping_range")?;
        let new=clipped.slice(current).ok_or("body_admission.prepare.reject.mapping_range")?;
        if old!=new {return Err("body_admission.prepare.reject.mapping_text_mismatch");}
        if original_cursor.is_some_and(|previous|previous!=mapped_start) {
            return Err("body_admission.prepare.reject.mapping_not_contiguous");
        }
        first.get_or_insert(mapped_start);
        original_cursor=Some(mapped_end);
        cursor=end;
        // Preserve ordered, clipped geometry in the key. On any later failure
        // this partial digest is discarded; no State or semantic effect occurs.
        witness(clipped, SourceOrigin::exact(mapped,None))
            .map_err(|_|"body_admission.prepare.reject.digest")?;
    }
    if cursor!=range.end {return Err("body_admission.prepare.reject.mapping_shape");}
    Ok(TextRange::new(first.ok_or("body_admission.prepare.reject.mapping_shape")?,
        original_cursor.ok_or("body_admission.prepare.reject.mapping_shape")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exact(v: (usize,usize),o: (usize,usize))->SourceMapSegment {
        SourceMapSegment::new(TextRange::new(v.0,v.1),SourceOrigin::exact(TextRange::new(o.0,o.1),None))
    }
    fn prove(old:&str,new:&str,range:TextRange,pieces:&[SourceMapSegment])->Result<TextRange,&'static str> {
        prove_extent(old,new,range,pieces,|_,_|Ok(()))
    }
    #[test]
    fn exact_extent_clips_and_streams_unicode_crlf_without_flattening() {
        let text="a😀\r\nbé\n";
        let pieces=[exact((0,5),(0,5)),exact((5,7),(5,7)),exact((7,11),(7,11))];
        let mut witness=Vec::new();
        assert_eq!(prove_extent(text,text,TextRange::new(1,10),&pieces,|v,o|{witness.push((v,o));Ok(())}),Ok(TextRange::new(1,10)));
        assert_eq!(witness.len(),3);
        assert_eq!(witness[0].0,TextRange::new(1,5));
        assert_eq!(witness[2].1.original_range,TextRange::new(7,10));
        assert_eq!(prove("xxabczz","abc",TextRange::new(0,3),&[exact((0,1),(2,3)),exact((1,3),(3,5))]),Ok(TextRange::new(2,5)));
    }
    #[test]
    fn exact_extent_rejects_generated_ambiguous_missing_duplicate_and_reordered_pieces() {
        let range=TextRange::new(0,4);
        let mut replaced=exact((0,4),(0,4));replaced.origins[0].kind=OriginKind::Replaced;
        let mut expanded=exact((0,4),(0,4));expanded.origins[0].expansion=Some(skript_parser::ExpansionId::new(1));
        let mut ambiguous=exact((0,4),(0,4));ambiguous.origins.push(ambiguous.origins[0]);
        let mut anchored=exact((0,4),(0,4));anchored.origins[0].kind=OriginKind::Anchored;
        for pieces in [vec![replaced],vec![expanded],vec![anchored],vec![ambiguous],vec![],
            vec![exact((0,1),(0,1)),exact((2,4),(2,4))],
            vec![exact((0,3),(0,3)),exact((2,4),(2,4))],
            vec![exact((0,2),(0,2)),exact((0,2),(0,2)),exact((2,4),(2,4))],
            vec![exact((2,4),(2,4)),exact((0,2),(0,2))],
            vec![exact((0,2),(2,4)),exact((2,4),(0,2))]] {
            assert!(prove("aaaa","aaaa",range,&pieces).is_err(),"{pieces:?}");
        }
        assert!(prove("abcd","abXd",range,&[exact((0,4),(0,4))]).is_err());
        assert!(prove("abcdef","abef",range,&[exact((0,2),(0,2)),exact((2,4),(4,6))]).is_err());
    }
    #[test]
    fn exact_extent_rejects_bad_byte_boundaries_bounds_and_empty_ranges() {
        for range in [TextRange::new(1,2),TextRange::new(0,5),TextRange::empty(0)] {
            assert!(prove("😀","😀",range,&[exact((0,4),(0,4))]).is_err());
        }
        assert!(prove("😀a","aa",TextRange::new(0,2),&[exact((0,2),(1,3))]).is_err());
        assert!(prove("aaaa","aaaa",TextRange::new(0,4),&[exact((0,4),(usize::MAX-1,usize::MAX))]).is_err());
    }
}
