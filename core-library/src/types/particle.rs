//! Skript's finite particle literal with an optional display count.

use super::registered_literal::{candidate_from_option, matching_option};
use crate::expression_candidates::metadata;
use crate::nlaocs::skript_parser_addon::types::{ExpressionLeafCandidate, ExpressionPayload};

const PARTICLE: &str = "org.skriptlang.skript.bukkit.particles.particleeffects.ParticleEffect";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.particle",
    classes: &[PARTICLE],
    parse,
    unresolved: None,
    all_type_options: false,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let (count, name, prefix_bytes) = split_count(text)?;
    let literal_start = payload
        .remaining
        .start
        .checked_add(u64::try_from(prefix_bytes).ok()?)?;
    let option = matching_option(payload, name, PARTICLE, literal_start, end)?;
    let mut parsed = candidate_from_option(
        &option,
        "core.literal.particle",
        payload.remaining.start,
        end,
    );
    parsed
        .metadata
        .push(metadata("particle-count", &count.to_string()));
    Some(parsed)
}

fn split_count(source: &str) -> Option<(i32, &str, usize)> {
    let Some((prefix, name)) = source.split_once(' ') else {
        return (!source.is_empty()).then_some((1, source, 0));
    };
    if prefix.is_empty() || !prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some((1, source, 0));
    }
    let count = prefix.parse::<i32>().ok()?.clamp(0, 16_384);
    (!name.is_empty()).then_some((count, name, prefix.len() + 1))
}

#[cfg(test)]
mod tests {
    use super::split_count;

    #[test]
    fn count_is_optional_unsigned_and_clamped() {
        assert_eq!(
            split_count("flame particle"),
            Some((1, "flame particle", 0))
        );
        assert_eq!(
            split_count("3 flame particle"),
            Some((3, "flame particle", 2))
        );
        assert_eq!(
            split_count("20000 flame particle"),
            Some((16_384, "flame particle", 6))
        );
        assert_eq!(
            split_count("-1 flame particle"),
            Some((1, "-1 flame particle", 0))
        );
        assert_eq!(split_count("999999999999999999 flame particle"), None);
    }
}
