//! Bounded structural access to exact JSON array records. No domain semantics.
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use std::{collections::{BTreeMap, BTreeSet}, fmt, ops::Range, sync::Arc};

const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ROWS: usize = 8192;
const MAX_OBJECT_KEYS: usize = 4096;

/// Invalid structure and resource exhaustion are distinct from an absent path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JsonPageError {
    #[error("invalid JSON array path, object, or page request")]
    Invalid,
    #[error("JSON array source, record, or response exceeds its bound")]
    Limit,
}

struct Object<'a>(BTreeMap<String, &'a RawValue>);
impl<'de> Deserialize<'de> for Object<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Object<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("an object with unique keys") }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut fields = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if fields.len() >= MAX_OBJECT_KEYS || fields.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate key or object key limit"));
                    }
                    fields.insert(key, map.next_value::<&'de RawValue>()?);
                }
                Ok(Object(fields))
            }
        }
        deserializer.deserialize_map(Fields)
    }
}
struct Array<'a>(Vec<&'a RawValue>);
impl<'de> Deserialize<'de> for Array<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Rows;
        impl<'de> Visitor<'de> for Rows {
            type Value = Array<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a bounded JSON array") }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut rows = Vec::new();
                while let Some(row) = seq.next_element::<&'de RawValue>()? {
                    if rows.len() == MAX_ROWS { return Err(serde::de::Error::custom("array row limit")); }
                    rows.push(row);
                }
                Ok(Array(rows))
            }
        }
        deserializer.deserialize_seq(Rows)
    }
}

/// Source-owned bytes plus exact row ranges. Cloning this index shares the bytes.
#[derive(Clone, Debug)]
pub struct JsonArrayIndex { bytes: Arc<[u8]>, rows: Arc<[Range<usize>]> }
/// One source index and raw or explicitly projected record bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPageRecord { pub index: u64, pub json: Vec<u8> }
/// A complete bounded page; an oversized first record is an error, not EOF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonArrayPage { pub total_records: u64, pub items: Vec<JsonPageRecord>, pub next_offset: Option<u64> }

impl JsonArrayIndex {
    /// A path consists of literal, decoded object keys. No expression evaluation.
    pub fn new(bytes: Arc<[u8]>, path: &[String]) -> Result<Option<Self>, JsonPageError> {
        if bytes.len() > MAX_DOCUMENT_BYTES { return Err(JsonPageError::Limit); }
        if path.len() > 8 || path.iter().any(|s| s.is_empty() || s.len() > 128) { return Err(JsonPageError::Invalid); }
        let mut value: &RawValue = serde_json::from_slice(&bytes).map_err(|_| JsonPageError::Invalid)?;
        for key in path {
            let object: Object<'_> = serde_json::from_str(value.get()).map_err(|_| JsonPageError::Invalid)?;
            let Some(found) = object.0.get(key) else { return Ok(None); };
            value = found;
        }
        let Array(values) = serde_json::from_str::<Array<'_>>(value.get()).map_err(|_| JsonPageError::Invalid)?;
        let base = bytes.as_ptr() as usize;
        let mut rows = Vec::with_capacity(values.len());
        for row in values {
            let start = (row.get().as_ptr() as usize).checked_sub(base).ok_or(JsonPageError::Invalid)?;
            let end = start.checked_add(row.get().len()).filter(|end| *end <= bytes.len()).ok_or(JsonPageError::Invalid)?;
            rows.push(start..end);
        }
        Ok(Some(Self { bytes, rows: rows.into() }))
    }
    /// Logical owned index bytes; the immutable source Arc is not copied.
    pub fn index_bytes(&self) -> usize { self.rows.len().saturating_mul(std::mem::size_of::<Range<usize>>()) }
    /// Empty fields returns exact raw rows. Other fields preserve exact values.
    pub fn page(&self, fields: &[String], offset: u64, limit: u32, max_bytes: u32) -> Result<JsonArrayPage, JsonPageError> {
        if limit == 0 || limit > 256 || max_bytes == 0 || max_bytes > 64 * 1024
            || fields.len() > 16 || fields.iter().any(|s| s.is_empty() || s.len() > 128)
            || fields.iter().collect::<BTreeSet<_>>().len() != fields.len() { return Err(JsonPageError::Invalid); }
        let start = usize::try_from(offset).map_err(|_| JsonPageError::Invalid)?;
        if start > self.rows.len() { return Err(JsonPageError::Invalid); }
        let end = start.saturating_add(limit as usize).min(self.rows.len());
        let mut items = Vec::new(); let mut used = 0usize;
        for index in start..end {
            let raw = &self.bytes[self.rows[index].clone()];
            // Bound raw-row decode work even when a projection would be small.
            if raw.len() > 64 * 1024 { return Err(JsonPageError::Limit); }
            let json = if fields.is_empty() { raw.to_vec() } else {
                let Object(object) = serde_json::from_slice(raw).map_err(|_| JsonPageError::Invalid)?;
                let selected = fields.iter().filter_map(|field| object.get(field).map(|value| (field, *value))).collect::<BTreeMap<_, _>>();
                serde_json::to_vec(&selected).map_err(|_| JsonPageError::Invalid)?
            };
            let bytes = json.len().saturating_add(8); // Include each returned source index.
            if used.saturating_add(bytes) > max_bytes as usize {
                if items.is_empty() { return Err(JsonPageError::Limit); }
                break;
            }
            used += bytes; items.push(JsonPageRecord { index: index as u64, json });
        }
        let next = start + items.len();
        Ok(JsonArrayPage { total_records: self.rows.len() as u64, items,
            next_offset: (next < self.rows.len()).then_some(next as u64) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn index(raw: &[u8], path: &[&str]) -> Result<Option<JsonArrayIndex>, JsonPageError> {
        JsonArrayIndex::new(Arc::from(raw), &path.iter().map(|s|s.to_string()).collect::<Vec<_>>())
    }
    #[test]
    fn nested_raw_pages_keep_bytes_indices_and_progress() {
        let raw=br#"{"x":{"a":[ { "n":18446744073709551617 },{"n":0.123456789012345678901234567890},{}]}}"#;
        let idx=index(raw,&["x","a"]).unwrap().unwrap();
        let first=idx.page(&[],0,1,1024).unwrap();assert_eq!(first.total_records,3);assert_eq!(first.next_offset,Some(1));
        assert_eq!(first.items[0].json,br#"{ "n":18446744073709551617 }"#);
        let second=idx.page(&[],1,2,1024).unwrap();assert_eq!(second.items[0].index,1);assert_eq!(second.next_offset,None);
        assert!(idx.page(&[],3,1,1024).unwrap().items.is_empty());assert_eq!(idx.page(&[],4,1,1024),Err(JsonPageError::Invalid));
        assert!(idx.page(&[],0,1,8).is_err());
    }
    #[test]
    fn projected_fields_keep_missing_null_unicode_and_wide_values() {
        let idx=index("{\"é.😀\":[{\"empty\":null,\"wide\":18446744073709551617,\"nested\":{\"n\":1e200},\"ignore\":false},{}]}".as_bytes(),&["é.😀"]).unwrap().unwrap();
        let p=idx.page(&["empty".into(),"missing".into(),"wide".into(),"nested".into()],0,64,32768).unwrap();
        let text=String::from_utf8(p.items[0].json.clone()).unwrap();assert!(text.contains("18446744073709551617"));assert!(text.contains("1e200"));assert!(text.contains("\"empty\":null"));assert!(!text.contains("missing"));assert_eq!(p.items[1].json,b"{}");
    }
    #[test]
    fn ambiguous_paths_and_fields_are_rejected_and_absence_is_distinct() {
        assert!(index(br#"{"x":[],"x":[]}"#,&["x"]).is_err());
        assert!(index(br#"{"other":[]}"#,&["missing"]).unwrap().is_none());
        assert!(index(br#"{"x":null}"#,&["x"]).is_err());
        let idx=index(br#"[{"x":1,"x":2}]"#,&[]).unwrap().unwrap();assert!(idx.page(&["x".into()],0,1,1024).is_err());
        assert!(idx.page(&["x".into(),"x".into()],0,1,1024).is_err());
    }
    #[test]
    fn response_shortening_and_resource_limits_fail_closed() {
        let idx=index(br#"[{"x":1},{"x":2}]"#,&[]).unwrap().unwrap();
        let p=idx.page(&[],0,64,15).unwrap();assert_eq!(p.items.len(),1);assert_eq!(p.next_offset,Some(1));
        assert!(idx.page(&[],0,0,1024).is_err());assert!(idx.page(&[],0,1,65537).is_err());
        assert!(index(br#"[]"#,&["a";9]).is_err());
        let many=format!("[{}]",vec!["null";MAX_ROWS+1].join(","));assert!(index(many.as_bytes(),&[]).is_err());
        assert_eq!(index(&vec![b' ';MAX_DOCUMENT_BYTES+1],&[]).unwrap_err(),JsonPageError::Limit);
    }
}
