//! Skript's EntityType literal: an EntityData value with an optional amount.
//!
//! Source: Skript 2.6.4, 2.15.4 and 2.16.0, `EntityType.parse` and `Utils.parseInt`.
//! <https://github.com/SkriptLang/Skript/blob/2.16.0/src/main/java/ch/njol/skript/entity/EntityType.java>

use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const ENTITY_TYPE: &str = "ch.njol.skript.entity.EntityType";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.entity-type",
    classes: &[ENTITY_TYPE],
    parse,
    unresolved: None,
    all_type_options: true,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    let active_type = payload.active_type.as_ref()?;
    if !payload.allow_literals || active_type.class_name != ENTITY_TYPE {
        return None;
    }
    let (raw_amount, description) = split_prefix(text);
    let start = payload
        .remaining
        .start
        .checked_add((text.len() - description.len()) as u64)?;
    let data =
        super::entity_data::parse_without_indefinite_article(payload, description, start, end)?;
    // The amount controls spawning at runtime. It does not make this Literal
    // a list of Expressions, even for `0 creepers` or `3 creepers`.
    let amount = if raw_amount == -1 { 1 } else { raw_amount };
    let mut parsed = candidate(
        "core.literal.entity-type",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        ENTITY_TYPE,
        DynamicMultiplicity::Single,
    );
    let data_metadata = data
        .metadata
        .iter()
        .map(|entry| {
            (
                entry.key.clone(),
                serde_json::Value::String(entry.value.clone()),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    parsed.metadata = vec![
        metadata("type-code-name", &active_type.code_name),
        metadata("type-definition-id", &active_type.definition_id),
        metadata("type-registration-id", &active_type.registration_id),
        metadata("entity-type-amount", &amount.to_string()),
        metadata("entity-type-raw-amount", &raw_amount.to_string()),
        // Retain the embedded type's supplier identity and source range without
        // mislabelling the enclosing EntityType as an EntityData literal.
        metadata(
            "entity-data",
            &serde_json::Value::Object(data_metadata).to_string(),
        ),
    ];
    Some(parsed)
}

fn split_prefix(text: &str) -> (i32, &str) {
    if let Some((prefix, description)) = text.split_once(' ')
        && !description.is_empty()
        && !description.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'])
    {
        // Java's unflagged `\d` is ASCII; Utils.parseInt clamps overflow.
        if !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit()) {
            return (prefix.parse::<i32>().unwrap_or(i32::MAX), description);
        }
        if prefix.eq_ignore_ascii_case("a") || prefix.eq_ignore_ascii_case("an") {
            return (-1, description);
        }
    }
    (-1, text)
}

#[cfg(test)]
mod tests {
    use super::split_prefix;

    #[test]
    fn quantity_is_a_nonnegative_ascii_integer_with_java_overflow_semantics() {
        for (text, expected) in [
            ("3 creepers", (3, "creepers")),
            ("0 creepers", (0, "creepers")),
            ("0001 creeper", (1, "creeper")),
            ("2147483647 creepers", (i32::MAX, "creepers")),
            ("2147483648 creepers", (i32::MAX, "creepers")),
            ("999999999999999999999 creepers", (i32::MAX, "creepers")),
            ("000000000000000000000001 creeper", (1, "creeper")),
        ] {
            assert_eq!(split_prefix(text), expected, "{text}");
        }
    }

    #[test]
    fn articles_and_absent_quantities_preserve_the_unspecified_amount() {
        assert_eq!(split_prefix("creeper"), (-1, "creeper"));
        assert_eq!(split_prefix("creepers"), (-1, "creepers"));
        assert_eq!(split_prefix("a creeper"), (-1, "creeper"));
        assert_eq!(split_prefix("AN enderman"), (-1, "enderman"));
    }

    #[test]
    fn prefix_parser_does_not_apply_number_or_item_type_grammar() {
        for text in [
            "-1 creepers",
            "+3 creepers",
            "1.5 creepers",
            "1_000 creepers",
            "\u{ff13} creepers",
            "3\tcreepers",
            "3\u{a0}creepers",
            "3 ",
            "3 creepers\n",
            "3 creepers\r",
            "3 creepers\u{85}",
            "3 creepers\u{2028}",
            "3 creepers\u{2029}",
        ] {
            assert_eq!(split_prefix(text), (-1, text), "{text:?}");
        }
        // Only the prefix is consumed here; EntityData decides whether the
        // remainder is valid. In particular, we must not strip a second article.
        assert_eq!(split_prefix("3 a creeper"), (3, "a creeper"));
        // The full Expression parser can match `3 of creepers` as ExprXOf;
        // that separate syntax must not become part of EntityType's grammar.
        assert_eq!(split_prefix("3 of creepers"), (3, "of creepers"));
        assert_eq!(split_prefix("3  creepers"), (3, " creepers"));
    }
}
