use crate::expression_candidates::candidate;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }

    let allow_angle_units = crate::runtime::skript_at_least(2, 10).unwrap_or(true);
    let shape = parse_number_shape(
        text,
        allow_angle_units,
        crate::runtime::skript_at_least(2, 12).unwrap_or(true),
    )?;
    let return_type = number_class(
        shape,
        payload
            .expected_types
            .iter()
            .map(|expected| expected.class_name.as_str()),
    )?;
    Some(candidate(
        "core.literal.number",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        return_type,
        DynamicMultiplicity::Single,
    ))
}

fn number_class<'a>(
    shape: NumberShape,
    expected_types: impl IntoIterator<Item = &'a str>,
) -> Option<&'static str> {
    let mut saw_specific_numeric_type = false;
    for expected in expected_types {
        let class = match expected {
            "java.lang.Long" => {
                saw_specific_numeric_type = true;
                (!shape.is_double() && shape.fits_i64).then_some("java.lang.Long")
            }
            "java.lang.Double" => Some("java.lang.Double"),
            "java.lang.Float" => {
                saw_specific_numeric_type = true;
                shape.fits_f32.then_some("java.lang.Float")
            }
            "java.lang.Short" => {
                saw_specific_numeric_type = true;
                (!shape.is_double() && shape.fits_i16).then_some("java.lang.Short")
            }
            "java.lang.Byte" => {
                saw_specific_numeric_type = true;
                (!shape.is_double() && shape.fits_i8).then_some("java.lang.Byte")
            }
            "java.lang.Integer" => {
                saw_specific_numeric_type = true;
                (!shape.is_double() && shape.fits_i32).then_some("java.lang.Integer")
            }
            "java.lang.Number" | "java.lang.Object" => return general_number_class(shape),
            _ => continue,
        };
        if class.is_some() {
            return class;
        }
    }
    (!saw_specific_numeric_type)
        .then(|| general_number_class(shape))
        .flatten()
}

fn general_number_class(shape: NumberShape) -> Option<&'static str> {
    if !shape.is_double() && shape.fits_i64 {
        Some("java.lang.Long")
    } else {
        Some("java.lang.Double")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NumberShape {
    fractional: bool,
    percent: bool,
    exponent: bool,
    radians: bool,
    fits_i32: bool,
    fits_i64: bool,
    fits_i16: bool,
    fits_i8: bool,
    fits_f32: bool,
}

impl NumberShape {
    fn is_double(self) -> bool {
        self.fractional || self.percent || self.exponent || self.radians
    }
}

/// Checks the grammar used by Skript's `NumberParser` without accepting Rust's
/// broader floating-point syntax such as `+1`, `NaN`, or hexadecimal numbers.
///
/// The two feature flags correspond to the upstream changes that added angle
/// units in 2.10 and scientific notation in 2.12. Keeping them explicit makes
/// the old and modern grammars testable without changing the active runtime
/// profile.
fn parse_number_shape(
    input: &str,
    allow_angle_units: bool,
    allow_scientific: bool,
) -> Option<NumberShape> {
    if input.is_empty() {
        return None;
    }

    let bytes = input.as_bytes();
    let mut cursor = if bytes.first() == Some(&b'-') { 1 } else { 0 };
    let allow_underscores = allow_angle_units || allow_scientific;
    if cursor == bytes.len() || !consume_digits(bytes, &mut cursor, allow_underscores) {
        return None;
    }

    let mut fractional = false;
    if bytes.get(cursor) == Some(&b'.') {
        fractional = true;
        cursor += 1;
        if !consume_digits(bytes, &mut cursor, allow_underscores) {
            return None;
        }
    }

    let percent = if bytes.get(cursor) == Some(&b'%') {
        cursor += 1;
        true
    } else {
        false
    };

    let exponent = if allow_scientific
        && bytes
            .get(cursor)
            .is_some_and(|byte| *byte == b'e' || *byte == b'E')
    {
        cursor += 1;
        if bytes
            .get(cursor)
            .is_some_and(|byte| *byte == b'+' || *byte == b'-')
        {
            cursor += 1;
        }
        if !consume_digits(bytes, &mut cursor, false) {
            return None;
        }
        true
    } else {
        false
    };

    // The Java parser's captured numeric group includes the exponent. Thus a
    // spelling such as `1%e2` matches the outer regex but fails conversion;
    // it is not a usable Number literal.
    if percent && exponent {
        return None;
    }

    let numeric_end = cursor;
    let radians = if cursor == bytes.len() {
        false
    } else {
        if !allow_angle_units {
            return None;
        }
        let is_radians = match input.get(cursor..)? {
            " rad" | " rads" | " radian" | " radians" | " in rad" | " in rads" | " in radian"
            | " in radians" => true,
            " deg" | " degs" | " degree" | " degrees" | " in deg" | " in degs" | " in degree"
            | " in degrees" => false,
            _ => return None,
        };
        cursor = bytes.len();
        is_radians
    };

    if cursor != bytes.len() {
        return None;
    }

    let mut numeric = input[..numeric_end].replace('_', "");
    if percent {
        numeric.pop();
    }
    let fits_i32 = numeric.parse::<i32>().is_ok();
    let fits_i64 = numeric.parse::<i64>().is_ok();
    let fits_i16 = numeric.parse::<i16>().is_ok();
    let fits_i8 = numeric.parse::<i8>().is_ok();
    let value = numeric.parse::<f64>().ok()?;
    let value = if percent { value / 100.0 } else { value };
    value.is_finite().then_some(NumberShape {
        fractional,
        percent,
        exponent,
        radians,
        fits_i32,
        fits_i64,
        fits_i16,
        fits_i8,
        fits_f32: value >= -(f32::MAX as f64) && value <= f32::MAX as f64,
    })
}

fn consume_digits(bytes: &[u8], cursor: &mut usize, allow_underscores: bool) -> bool {
    let mut saw_digit = false;
    let mut previous_was_digit = false;
    while let Some(byte) = bytes.get(*cursor) {
        if byte.is_ascii_digit() {
            saw_digit = true;
            previous_was_digit = true;
            *cursor += 1;
        } else if allow_underscores
            && *byte == b'_'
            && previous_was_digit
            && bytes.get(*cursor + 1).is_some_and(u8::is_ascii_digit)
        {
            previous_was_digit = false;
            *cursor += 1;
        } else {
            break;
        }
    }
    saw_digit && previous_was_digit
}

#[cfg(test)]
mod tests {
    use super::{number_class, parse_number_shape};

    #[test]
    fn legacy_number_grammar_rejects_modern_spellings() {
        assert!(parse_number_shape("-42", false, false).is_some());
        assert!(parse_number_shape("2.5%", false, false).is_some());
        assert!(parse_number_shape("1_000", false, false).is_none());
        assert!(parse_number_shape("1e3", false, false).is_none());
        assert!(parse_number_shape("1 rad", false, false).is_none());
        assert!(parse_number_shape("+1", false, false).is_none());
        assert!(parse_number_shape("1.", false, false).is_none());
    }

    #[test]
    fn modern_number_grammar_accepts_units_exponents_and_separators() {
        assert!(parse_number_shape("1_000", true, true).is_some());
        assert!(parse_number_shape("1.25e-2", true, true).is_some());
        assert!(
            parse_number_shape("3.14 radians", true, true)
                .is_some_and(|shape| shape.is_double() && shape.radians)
        );
        assert!(parse_number_shape("50%", true, true).is_some_and(|shape| shape.percent));
        assert!(parse_number_shape("1%e2", true, true).is_none());
    }

    #[test]
    fn degree_integer_keeps_the_integer_result_shape() {
        let degree = parse_number_shape("90 degrees", true, true).unwrap();
        assert!(!degree.is_double());
        assert_eq!(number_class(degree, []), Some("java.lang.Long"));
        assert_eq!(
            number_class(degree, ["java.lang.Integer"]),
            Some("java.lang.Integer")
        );
        assert_eq!(
            number_class(degree, ["java.lang.Double"]),
            Some("java.lang.Double")
        );
        assert!(parse_number_shape("1 rad", true, true).is_some_and(|shape| shape.is_double()));
    }

    #[test]
    fn general_integer_resolution_follows_skript_class_info_order() {
        let above_integer = parse_number_shape("2147483648", true, true).unwrap();
        assert_eq!(number_class(above_integer, []), Some("java.lang.Long"));
        assert_eq!(number_class(above_integer, ["java.lang.Integer"]), None);

        let above_long = parse_number_shape("9223372036854775808", true, true).unwrap();
        assert_eq!(number_class(above_long, []), Some("java.lang.Double"));
    }
}
