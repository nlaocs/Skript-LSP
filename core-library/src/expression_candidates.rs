use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
    MetadataEntry, TextRange,
};

pub(crate) fn parse(payload: &ExpressionPayload) -> Option<ExpressionLeafCandidate> {
    for end in payload.candidate_ends.iter().copied().rev() {
        let Some(text) = expression_slice(payload, end) else {
            continue;
        };
        if let Some(candidate) = crate::primitives::parse(payload, text, end)
            .or_else(|| crate::types::parse(payload, text, end))
        {
            return Some(candidate);
        }
    }
    None
}

pub(crate) fn expression_slice(payload: &ExpressionPayload, end: u64) -> Option<&str> {
    let start = usize::try_from(payload.remaining.start).ok()?;
    let end = usize::try_from(end).ok()?;
    let remaining_end = usize::try_from(payload.remaining.end).ok()?;
    (start <= end && end <= remaining_end)
        .then(|| payload.input.get(start..end))
        .flatten()
}

pub(crate) fn candidate(
    parser_id: &str,
    kind: ExpressionLeafKind,
    start: u64,
    end: u64,
    return_type: &str,
    multiplicity: DynamicMultiplicity,
) -> ExpressionLeafCandidate {
    ExpressionLeafCandidate {
        parser_id: parser_id.to_owned(),
        kind,
        range: TextRange { start, end },
        return_type: Some(return_type.to_owned()),
        multiplicity: Some(multiplicity),
        children: Vec::new(),
        metadata: Vec::new(),
    }
}

pub(crate) fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}
