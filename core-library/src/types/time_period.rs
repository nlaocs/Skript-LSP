//! Skript's named and ranged in-game time period parser.

use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const TIME_PERIOD: &str = "ch.njol.skript.util.Timeperiod";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.time-period",
    classes: &[TIME_PERIOD],
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
    let (start, finish) = parse_range(text)?;
    let mut parsed = candidate(
        "core.literal.time-period",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        TIME_PERIOD,
        DynamicMultiplicity::Single,
    );
    parsed.metadata.extend([
        metadata("time-period-start-ticks", &start.to_string()),
        metadata("time-period-end-ticks", &finish.to_string()),
    ]);
    Some(parsed)
}

fn parse_range(source: &str) -> Option<(i32, i32)> {
    for (name, range) in [
        ("day", (0, 11_999)),
        ("dusk", (12_000, 13_799)),
        ("night", (13_800, 22_199)),
        ("dawn", (22_200, 23_999)),
    ] {
        if source.eq_ignore_ascii_case(name) {
            return Some(range);
        }
    }
    if let Some((start, end)) = source.split_once('-') {
        return Some((
            super::time::parse_ticks(java_trim(start))?,
            super::time::parse_ticks(java_trim(end))?,
        ));
    }
    let time = super::time::parse_ticks(source)?;
    Some((time, time))
}

fn java_trim(source: &str) -> &str {
    source.trim_matches(|character| character <= '\u{20}')
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn parses_named_single_and_ranged_periods() {
        assert_eq!(parse_range("day"), Some((0, 11_999)));
        assert_eq!(parse_range("DUSK"), Some((12_000, 13_799)));
        assert_eq!(parse_range("10:00"), Some((4_000, 4_000)));
        assert_eq!(parse_range("10:00 - 12:00"), Some((4_000, 6_000)));
        assert_eq!(parse_range("10:00-25:00"), None);
    }
}
