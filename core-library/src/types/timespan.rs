use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const TIMESPAN: &str = "ch.njol.skript.util.Timespan";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.timespan",
    classes: &["ch.njol.skript.util.Timespan"],
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
    let command = payload.context.values.iter().any(|entry| {
        entry.key == "parser.parse-context" && entry.value.eq_ignore_ascii_case("COMMAND")
    });
    let millis = parse_english(text, command)?;
    let mut parsed = candidate(
        "core.literal.timespan",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        TIMESPAN,
        DynamicMultiplicity::Single,
    );
    parsed
        .metadata
        .push(metadata("timespan-milliseconds", &millis.to_string()));
    Some(parsed)
}

fn parse_english(source: &str, command: bool) -> Option<u64> {
    if source.is_empty() {
        return None;
    }
    parse_clock(source).or_else(|| parse_units(source, command))
}

fn parse_clock(source: &str) -> Option<u64> {
    let (clock, fractional) = match source.split_once('.') {
        Some((clock, fractional)) => {
            if fractional.is_empty()
                || fractional.len() > 4
                || !fractional.bytes().all(|byte| byte.is_ascii_digit())
                || fractional.contains('.')
            {
                return None;
            }
            (clock, Some(fractional))
        }
        None => (source, None),
    };
    let parts = clock.split(':').collect::<Vec<_>>();
    if !(2..=4).contains(&parts.len())
        || parts[0].is_empty()
        || !parts[0].bytes().all(|byte| byte.is_ascii_digit())
        || parts[1..]
            .iter()
            .any(|part| part.len() != 2 || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let units = [86_400_000_u64, 3_600_000, 60_000, 1_000];
    let offset = units.len().checked_sub(parts.len())?;
    let mut total = 0_u64;
    for (part, unit) in parts.iter().zip(&units[offset..]) {
        total = total.checked_add(part.parse::<u64>().ok()?.checked_mul(*unit)?)?;
    }
    if let Some(fractional) = fractional {
        // Skript parses the suffix after `.` as an integer millisecond field;
        // it does not scale `.5` to 500 ms.
        total = total.checked_add(fractional.parse::<u64>().ok()?)?;
    }
    Some(total)
}

fn parse_units(source: &str, command: bool) -> Option<u64> {
    let words = source.split_ascii_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return None;
    }
    let mut total = 0_u64;
    let mut index = 0;
    let mut minecraft_time = false;
    let mut time_mode_set = false;
    let conjunction = crate::language::value("and", "and");
    let real_markers = language_list("time.real", "real, rl, irl");
    let minecraft_markers = language_list("time.minecraft", "mc, minecraft");
    while index < words.len() {
        if words[index].eq_ignore_ascii_case(&conjunction) {
            if index == 0 || index + 1 == words.len() {
                return None;
            }
            index += 1;
            continue;
        }

        let mut amount = 1_f64;
        let mut unit = words[index];
        if crate::language::is_indefinite_article(unit) {
            index += 1;
            unit = *words.get(index)?;
        } else if decimal_number(unit) {
            amount = unit.parse::<f64>().ok()?;
            index += 1;
            unit = *words.get(index)?;
        }

        if matches_owned_ignore_ascii_case(unit, &real_markers) {
            if time_mode_set && minecraft_time {
                return None;
            }
            index += 1;
            unit = *words.get(index)?;
        } else if matches_owned_ignore_ascii_case(unit, &minecraft_markers) {
            if time_mode_set && !minecraft_time {
                return None;
            }
            minecraft_time = true;
            index += 1;
            unit = *words.get(index)?;
        }

        unit = unit.strip_suffix(',').unwrap_or(unit);
        if command && amount == 1.0 {
            let digits = unit
                .bytes()
                .take_while(|byte| byte.is_ascii_digit() || *byte == b'.')
                .count();
            if digits > 0 && digits < unit.len() {
                let compact_amount = &unit[..digits];
                if decimal_number(compact_amount)
                    && unit[digits..]
                        .bytes()
                        .all(|byte| byte.is_ascii_alphabetic())
                {
                    amount = compact_amount.parse::<f64>().ok()?;
                    unit = &unit[digits..];
                }
            }
        }

        let unit_millis = unit_millis(unit)?;
        if minecraft_time && unit_millis != 50 {
            amount /= 72.0;
        }
        let value = (amount * unit_millis as f64).round();
        if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
            return None;
        }
        total = total.checked_add(value as u64)?;
        time_mode_set = true;
        index += 1;
    }
    Some(total)
}

fn decimal_number(value: &str) -> bool {
    let mut dot = false;
    !value.is_empty()
        && value.bytes().all(|byte| {
            if byte == b'.' && !dot {
                dot = true;
                true
            } else {
                byte.is_ascii_digit()
            }
        })
        && value != "."
        && !value.starts_with('.')
        && !value.ends_with('.')
}

fn unit_millis(unit: &str) -> Option<u64> {
    for (name, fallback_full, fallback_short, millis) in [
        ("millisecond", "millisecond¦s", "ms", 1),
        ("tick", "tick¦s", "t", 50),
        ("second", "second¦s", "s", 1_000),
        ("minute", "minute¦s", "m", 60_000),
        ("hour", "hour¦s", "h", 3_600_000),
        ("day", "day¦s", "d", 86_400_000),
        ("week", "week¦s", "w", 604_800_000),
        ("month", "month¦s", "mo", 2_592_000_000),
        ("year", "year¦s", "y", 31_536_000_000),
    ] {
        let full = crate::language::value(&format!("time.{name}.full"), fallback_full);
        let short = crate::language::value(&format!("time.{name}.short"), fallback_short);
        if noun_forms(&full)
            .into_iter()
            .chain(noun_forms(&short))
            .any(|form| unit.eq_ignore_ascii_case(&form))
        {
            return Some(millis);
        }
    }
    None
}

fn matches_owned_ignore_ascii_case(value: &str, candidates: &[String]) -> bool {
    candidates
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

fn language_list(key: &str, fallback: &str) -> Vec<String> {
    crate::language::value(key, fallback)
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn noun_forms(value: &str) -> [String; 2] {
    let value = value
        .rsplit_once('@')
        .map_or(value, |(value, _)| value)
        .trim();
    let marker_count = value.matches('¦').count();
    let mut remaining_markers = marker_count;
    let mut part = 3_u8;
    let mut singular = String::new();
    let mut plural = String::new();
    for (index, segment) in value.split('¦').enumerate() {
        if part & 1 != 0 {
            singular.push_str(segment);
        }
        if part & 2 != 0 {
            plural.push_str(segment);
        }
        if index < marker_count {
            part = if remaining_markers >= 2 {
                part % 3 + 1
            } else if part == 2 {
                3
            } else {
                2
            };
            remaining_markers -= 1;
        }
    }
    [singular, plural]
}

#[cfg(test)]
mod tests {
    use super::{noun_forms, parse_english};

    #[test]
    fn parses_skript_clock_forms() {
        assert_eq!(parse_english("01:30", false), Some(90_000));
        assert_eq!(parse_english("1:02:03", false), Some(3_723_000));
        assert_eq!(parse_english("1:02:03:04.5", false), Some(93_784_005));
        assert_eq!(parse_english("1:2", false), None);
    }

    #[test]
    fn parses_units_markers_and_command_short_forms() {
        assert_eq!(parse_english("2 seconds and 1 tick", false), Some(2_050));
        assert_eq!(parse_english("1.5 minutes", false), Some(90_000));
        assert_eq!(parse_english("72 minecraft seconds", false), Some(1_000));
        assert_eq!(parse_english("2s", false), None);
        assert_eq!(parse_english("2s", true), Some(2_000));
        assert_eq!(parse_english("minecraft second real tick", false), None);
    }

    #[test]
    fn parses_skript_noun_plural_markers() {
        assert_eq!(noun_forms("second¦s"), ["second", "seconds"]);
        assert_eq!(noun_forms("categor¦y¦ies @a"), ["category", "categories"]);
    }
}
