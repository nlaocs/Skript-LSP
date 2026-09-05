//! Skript's in-game clock parser.
//!
//! Source: `ch.njol.skript.util.Time.parse` in Skript 2.6.4 through 2.16.0.

use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const TIME: &str = "ch.njol.skript.util.Time";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.time",
    classes: &[TIME],
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
    let ticks = parse_ticks(text)?;
    let mut parsed = candidate(
        "core.literal.time",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        TIME,
        DynamicMultiplicity::Single,
    );
    parsed
        .metadata
        .push(metadata("time-ticks", &ticks.to_string()));
    Some(parsed)
}

pub(super) fn parse_ticks(source: &str) -> Option<i32> {
    parse_24_hour(source).or_else(|| parse_12_hour(source))
}

fn parse_24_hour(source: &str) -> Option<i32> {
    let (hours, minutes) = source.split_once(':')?;
    if minutes.contains(':') || minutes.len() != 2 {
        return None;
    }
    let mut hours = one_or_two_digits(hours)?;
    let minutes = two_digits(minutes)?;
    if hours == 24 {
        hours = 0;
    } else if hours > 24 {
        return None;
    }
    (minutes < 60).then(|| minecraft_ticks(hours, minutes))
}

fn parse_12_hour(source: &str) -> Option<i32> {
    if source.len() < 3 {
        return None;
    }
    let suffix = source.get(source.len() - 2..)?;
    let pm = if suffix.eq_ignore_ascii_case("am") {
        false
    } else if suffix.eq_ignore_ascii_case("pm") {
        true
    } else {
        return None;
    };
    let mut clock = source.get(..source.len() - 2)?;
    if let Some(without_space) = clock.strip_suffix(' ') {
        clock = without_space;
    }
    let (hours, minutes) = match clock.split_once(':') {
        Some((hours, minutes)) if !minutes.contains(':') => {
            (one_or_two_digits(hours)?, two_digits(minutes)?)
        }
        None => (one_or_two_digits(clock)?, 0),
        Some(_) => return None,
    };
    if hours > 12 || minutes >= 60 {
        return None;
    }
    let hours = (if hours == 12 { 0 } else { hours }) + if pm { 12 } else { 0 };
    Some(minecraft_ticks(hours, minutes))
}

fn one_or_two_digits(source: &str) -> Option<i32> {
    ((1..=2).contains(&source.len()) && source.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| source.parse().ok())?
}

fn two_digits(source: &str) -> Option<i32> {
    (source.len() == 2 && source.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| source.parse().ok())?
}

fn minecraft_ticks(hours: i32, minutes: i32) -> i32 {
    let raw = hours as f64 * 1_000.0 - 6_000.0 + minutes as f64 * (1_000.0 / 60.0);
    ((raw + 0.5).floor() as i32).rem_euclid(24_000)
}

#[cfg(test)]
mod tests {
    use super::parse_ticks;

    #[test]
    fn follows_skript_clock_ranges_and_day_offset() {
        assert_eq!(parse_ticks("0:00"), Some(18_000));
        assert_eq!(parse_ticks("24:59"), Some(18_983));
        assert_eq!(parse_ticks("8 pm"), Some(14_000));
        assert_eq!(parse_ticks("12:30AM"), Some(18_500));
        assert_eq!(parse_ticks("25:00"), None);
        assert_eq!(parse_ticks("13 pm"), None);
        assert_eq!(parse_ticks("8:60"), None);
    }
}
