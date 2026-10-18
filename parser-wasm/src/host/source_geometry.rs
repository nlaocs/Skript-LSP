//! Native pattern-capture geometry retained in the final, macro-editable AST.
//!
//! Existing WIT Span captures carry the positions. An owner-protected seal binds
//! their complete set to the selected node identity. AST macros may edit spans,
//! but an unchanged seal then stops certifying native source ownership. No WIT
//! layout or ABI changes are needed, and consumers must fail closed on mismatch.
use super::{AstNode, Capture, CaptureValue, MappedSpan, WitMetadataEntry};
use sha2::{Digest, Sha256};
use skript_parser::PatternCapture;
use std::collections::HashSet;

pub const PATTERN_CAPTURE_PREFIX: &str = "parser.pattern-capture:";
const SEAL: &str = "parser.pattern-capture-seal.v1";

/// Publish the selected native captures during initial AST projection only.
/// A consumer must never reseal an AST macro's modified output as native proof.
pub fn retain_pattern_captures(node: &mut AstNode, captures: &[PatternCapture]) {
    for capture in captures {
        let (PatternCapture::Regex { span, .. } | PatternCapture::TypeExpression { span, .. }) =
            capture;
        node.captures.push(Capture {
            name: format!("{PATTERN_CAPTURE_PREFIX}{}", capture.capture_index()),
            value: CaptureValue::Span(super::mapped_span_to_wit(span.mapped.clone())),
        });
    }
    node.metadata
        .retain(|entry| entry.owner_component_id.is_some() || entry.key != SEAL);
    if let Some(value) = fingerprint(node) {
        node.metadata.push(WitMetadataEntry {
            owner_component_id: None,
            key: SEAL.into(),
            value,
        });
    }
}

pub fn has_pattern_capture_geometry(node: &AstNode) -> bool {
    node.metadata
        .iter()
        .any(|entry| entry.owner_component_id.is_none() && entry.key == SEAL)
        || node
            .captures
            .iter()
            .any(|capture| capture.name.starts_with(PATTERN_CAPTURE_PREFIX))
}

/// False after removal, narrowing, remapping, or replacement of native evidence.
/// Node-ID canonicalization alone does not invalidate source positions.
pub fn pattern_capture_geometry_is_current(node: &AstNode) -> bool {
    let mut seals = node
        .metadata
        .iter()
        .filter(|entry| entry.owner_component_id.is_none() && entry.key == SEAL);
    let Some(seal) = seals.next() else {
        return false;
    };
    seals.next().is_none() && fingerprint(node).as_ref() == Some(&seal.value)
}

fn fingerprint(node: &AstNode) -> Option<String> {
    let mut digest = Sha256::new();
    text(&mut digest, "native-pattern-captures-v1");
    text(&mut digest, &format!("{:?}", node.kind));
    text(&mut digest, &node.syntax_id);
    text(&mut digest, &node.text);
    span(&mut digest, &node.span);
    text(
        &mut digest,
        &format!(
            "{:?}",
            node.summary.as_ref().map(|summary| (
                &summary.kind,
                &summary.definition_id,
                &summary.registration_id,
                summary.pattern_index
            ))
        ),
    );
    let mut slots = HashSet::new();
    let mut evidence = Vec::new();
    for capture in &node.captures {
        if let Some(index) = capture.name.strip_prefix(PATTERN_CAPTURE_PREFIX) {
            let index: u64 = index.parse().ok()?;
            if !slots.insert(index) || !matches!(capture.value, CaptureValue::Span(_)) {
                return None;
            }
        }
        // These values do not refer to arena IDs and survive canonicalization.
        if matches!(capture.value, CaptureValue::Span(_) | CaptureValue::Text(_)) {
            evidence.push(capture);
        }
    }
    evidence.sort_by(|left, right| left.name.cmp(&right.name));
    digest.update((evidence.len() as u64).to_le_bytes());
    for capture in evidence {
        text(&mut digest, &capture.name);
        match &capture.value {
            CaptureValue::Span(value) => {
                digest.update([0]);
                span(&mut digest, value);
            }
            CaptureValue::Text(value) => {
                digest.update([1]);
                text(&mut digest, value);
            }
            _ => unreachable!(),
        }
    }
    Some(format!("{:x}", digest.finalize()))
}

fn text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

fn span(digest: &mut Sha256, value: &MappedSpan) {
    digest.update(value.virtual_range.start.to_le_bytes());
    digest.update(value.virtual_range.end.to_le_bytes());
    digest.update((value.origins.len() as u64).to_le_bytes());
    for origin in &value.origins {
        digest.update(origin.original_range.start.to_le_bytes());
        digest.update(origin.original_range.end.to_le_bytes());
        text(digest, &format!("{:?}", origin.kind));
        text(digest, &format!("{:?}", origin.expansion));
    }
}
