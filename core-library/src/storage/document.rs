//! Bounded, source-fenced concatenation of retained Catalog document bytes.
//! UTF-8 is decoded only after all chunks have been joined.
use super::evidence::SourceIdentity;

pub(super) const MANIFEST_LIMIT: usize = 2 * 1024 * 1024;
pub(super) const CHUNK_BYTES: u32 = 64 * 1024;
const MAX_CHUNKS: usize = 64;

pub(super) struct Chunk {
    pub offset: u64,
    pub total_length: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadFailure { Unavailable, Source, Limit, Shape }

/// Errors (including cancellation/transport failure) must not be cached as a
/// permanent missing proof. The source digest covers the complete retained raw
/// source; snapshotId alone does not cover optional Manifest fields.
pub(super) fn read_bound(
    expected: &SourceIdentity,
    limit: usize,
    mut source: impl FnMut() -> Result<SourceIdentity, ReadFailure>,
    mut read: impl FnMut(u64, u32) -> Result<Option<Chunk>, ReadFailure>,
) -> Result<Option<Vec<u8>>, ReadFailure> {
    if limit == 0 || limit > MANIFEST_LIMIT { return Err(ReadFailure::Limit); }
    let mut bytes = Vec::new();
    let mut total = None;
    for _ in 0..MAX_CHUNKS {
        if &source()? != expected { return Err(ReadFailure::Source); }
        let offset = bytes.len();
        let requested = CHUNK_BYTES.min((limit - offset) as u32);
        if requested == 0 { return Err(ReadFailure::Shape); }
        let chunk = read(offset as u64, requested)?;
        if &source()? != expected { return Err(ReadFailure::Source); }
        let Some(chunk) = chunk else {
            return if offset == 0 { Ok(None) } else { Err(ReadFailure::Shape) };
        };
        let length = usize::try_from(chunk.total_length).map_err(|_| ReadFailure::Limit)?;
        if length > limit { return Err(ReadFailure::Limit); }
        if chunk.offset != offset as u64 || length == 0 || total.is_some_and(|n| n != length)
            || chunk.bytes.is_empty() || chunk.bytes.len() > requested as usize
            || offset.checked_add(chunk.bytes.len()).is_none_or(|end| end > length) {
            return Err(ReadFailure::Shape);
        }
        if total.is_none() {
            // The advertised total is checked before allocating for it.
            bytes.try_reserve_exact(length).map_err(|_| ReadFailure::Limit)?;
            total = Some(length);
        }
        bytes.extend_from_slice(&chunk.bytes);
        if bytes.len() == length { return Ok(Some(bytes)); }
    }
    Err(ReadFailure::Limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> SourceIdentity {
        SourceIdentity { snapshot_id: "a".repeat(64), source_digest: "b".repeat(64) }
    }
    fn chunk(data: &[u8], at: u64, max: u32) -> Chunk {
        Chunk { offset: at, total_length: data.len() as u64,
            bytes: data[at as usize..data.len().min(at as usize + max as usize)].to_vec() }
    }
    #[test]
    fn joins_real_manifest_in_seven_chunks_and_utf8_across_a_boundary() {
        for data in [include_bytes!("evidence/observation/data/real-capture/core-manifest.json").as_slice(),
            include_bytes!("evidence/observation/data/real-capture/reflect-manifest.json").as_slice()] {
            let mut calls=0;
            let bytes=read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|at,max|{
                calls+=1; Ok(Some(chunk(data,at,max)))
            }).unwrap().unwrap();
            assert_eq!(bytes,data); assert_eq!(calls,7);
            assert!(serde_json::from_slice::<serde_json::Value>(&data[..CHUNK_BYTES as usize]).is_err(),
                "the previous single-chunk read cannot decode this actual Manifest");
        }
        let data=format!("{}é", "a".repeat(CHUNK_BYTES as usize-1)).into_bytes();
        let bytes=read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|at,max|Ok(Some(chunk(&data,at,max)))).unwrap().unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap().chars().last(),Some('é'));
    }
    #[test]
    fn rejects_oversize_progress_offset_total_and_missing_chunks() {
        let data=vec![b' ';CHUNK_BYTES as usize+8];
        for change in ["oversize","empty","offset","total","missing","oversized-chunk"] {
            let result=read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|at,max|{
                if change=="missing" && at>0 {return Ok(None);}
                let mut c=chunk(&data,at,max);
                match change {
                    "oversize"=>c.total_length=MANIFEST_LIMIT as u64+1,
                    "empty"=>c.bytes.clear(), "offset"=>c.offset+=1,
                    "total" if at>0=>c.total_length+=1,
                    "oversized-chunk"=>c.bytes=vec![0;max as usize+1], _=>{}
                } Ok(Some(c))
            });
            assert!(result.is_err(),"{change}");
        }
        let mut calls=0;
        assert_eq!(read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|at,_|{
            calls+=1; Ok(Some(Chunk {offset:at,total_length:1000,bytes:vec![0]}))
        }),Err(ReadFailure::Limit));
        assert_eq!(calls,MAX_CHUNKS);
    }
    #[test]
    fn source_change_and_interruption_abort_without_a_negative_cache_result() {
        let data=vec![0;CHUNK_BYTES as usize+1];
        let mut calls=0;
        let result=read_bound(&source(),MANIFEST_LIMIT,||{
            calls+=1;let mut s=source();if calls==4{s.source_digest="c".repeat(64);}Ok(s)
        },|at,max|Ok(Some(chunk(&data,at,max))));
        assert_eq!(result,Err(ReadFailure::Source));
        let interrupted=read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|at,max|{
            if at==0 {Ok(Some(chunk(&data,at,max)))} else {Err(ReadFailure::Unavailable)}
        });
        assert_eq!(interrupted,Err(ReadFailure::Unavailable));
        assert_eq!(read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|at,max|Ok(Some(chunk(&data,at,max)))).unwrap(),Some(data));
        assert_eq!(read_bound(&source(),MANIFEST_LIMIT,||Ok(source()),|_,_|Ok(None)).unwrap(),None);
    }
}
