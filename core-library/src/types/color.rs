//! Skript's named and RGB color parser.

use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const COLOR: &str = "ch.njol.skript.util.Color";
const NAMED_COLORS: &[(&str, &str)] = &[
    ("black", "black"),
    ("dark_grey", "dark grey, dark gray"),
    ("light_grey", "grey, light grey, gray, light gray, silver"),
    ("white", "white"),
    ("dark_blue", "blue, dark blue"),
    ("brown", "brown, light blue, indigo"),
    (
        "dark_cyan",
        "cyan, aqua, dark cyan, dark aqua, dark turquoise, dark turquois",
    ),
    (
        "light_cyan",
        "light cyan, light aqua, turquoise, turquois, light turquoise, light turquois",
    ),
    ("dark_green", "green, dark green"),
    ("light_green", "light green, lime, lime green"),
    ("yellow", "yellow, light yellow"),
    ("orange", "orange, gold, dark yellow"),
    ("dark_red", "red, dark red"),
    ("light_red", "pink, light red"),
    ("dark_purple", "purple, dark purple"),
    ("light_purple", "magenta, light purple"),
];

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.color",
    classes: &[COLOR],
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
    if let Some((red, green, blue)) = parse_rgb(text) {
        let mut parsed = candidate(
            "core.literal.color-rgb",
            ExpressionLeafKind::Literal,
            payload.remaining.start,
            end,
            COLOR,
            DynamicMultiplicity::Single,
        );
        parsed.metadata.extend([
            metadata("color-red", &red.to_string()),
            metadata("color-green", &green.to_string()),
            metadata("color-blue", &blue.to_string()),
        ]);
        return Some(parsed);
    }
    if let Some(mut parsed) = super::registered_literal::parse(payload, end) {
        parsed.parser_id = "core.literal.color".to_owned();
        return Some(parsed);
    }
    let canonical = named_color(text)?;
    let mut parsed = candidate(
        "core.literal.color",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        COLOR,
        DynamicMultiplicity::Single,
    );
    parsed.metadata.push(metadata("color-name", canonical));
    Some(parsed)
}

fn parse_rgb(source: &str) -> Option<(u8, u8, u8)> {
    let values = source
        .strip_prefix("rgb ")
        .or_else(|| source.strip_prefix("RGB "))?;
    let mut values = values.split(", ");
    let red = color_channel(values.next()?)?;
    let green = color_channel(values.next()?)?;
    let blue = color_channel(values.next()?)?;
    values.next().is_none().then_some((red, green, blue))
}

fn color_channel(source: &str) -> Option<u8> {
    if source.is_empty() || !source.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Apache NumberUtils.toInt returns zero on integer overflow; ColorRGB clamps afterward.
    Some(source.parse::<i32>().unwrap_or(0).clamp(0, 255) as u8)
}

fn named_color(source: &str) -> Option<&'static str> {
    NAMED_COLORS.iter().find_map(|(name, fallback)| {
        crate::language::value(&format!("colors.{name}.names"), fallback)
            .split(',')
            .any(|candidate| source.eq_ignore_ascii_case(candidate.trim()))
            .then_some(*name)
    })
}

#[cfg(test)]
mod tests {
    use super::{named_color, parse_rgb};

    #[test]
    fn parses_exact_skript_rgb_shape_and_clamps_channels() {
        assert_eq!(parse_rgb("rgb 255, 0, 12"), Some((255, 0, 12)));
        assert_eq!(parse_rgb("RGB 999, 2, 3"), Some((255, 2, 3)));
        assert_eq!(parse_rgb("rgb 999999999999, 2, 3"), Some((0, 2, 3)));
        assert_eq!(parse_rgb("Rgb 1, 2, 3"), None);
        assert_eq!(parse_rgb("rgb 1,2,3"), None);
    }

    #[test]
    fn recognizes_language_backed_named_colors_without_a_supplier() {
        assert_eq!(named_color("dark gray"), Some("dark_grey"));
        assert_eq!(named_color("lime green"), Some("light_green"));
        assert_eq!(named_color("not a color"), None);
    }
}
