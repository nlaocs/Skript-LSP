use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
    ExpressionTypeOption,
};
#[cfg(not(target_arch = "wasm32"))]
use fancy_regex::Regex;
#[cfg(not(target_arch = "wasm32"))]
use std::cell::RefCell;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static USER_INPUT_PATTERNS: RefCell<HashMap<String, Option<Regex>>> =
        RefCell::new(HashMap::new());
}

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.class-info",
    classes: &["ch.njol.skript.classes.ClassInfo"],
    parse,
    unresolved: None,
    all_type_options: true,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let (option, plural) = super::match_user_type_option(text, &payload.type_options)?;
    let mut candidate = candidate(
        "core.literal.class-info",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        "ch.njol.skript.classes.ClassInfo",
        DynamicMultiplicity::Single,
    );
    candidate.metadata = vec![
        metadata("semantic-role", "target-type"),
        metadata("target-class", &option.class_name),
        metadata("type-code-name", &option.code_name),
        metadata("type-plural", if plural { "true" } else { "false" }),
        metadata(
            "has-parser",
            if option.has_parser { "true" } else { "false" },
        ),
        metadata(
            "has-supplier",
            if option.has_supplier { "true" } else { "false" },
        ),
    ];
    if !option.parse_contexts.is_empty() {
        candidate.metadata.push(metadata(
            "type-parse-contexts",
            &option.parse_contexts.join(","),
        ));
    }
    Some(candidate)
}

pub(super) fn type_option<'a>(
    name: &str,
    options: &'a [ExpressionTypeOption],
) -> Option<(&'a ExpressionTypeOption, bool)> {
    let name = name.trim();
    options
        .iter()
        .find(|option| name.eq_ignore_ascii_case(&option.code_name))
        .map(|option| (option, false))
        .or_else(|| {
            options
                .iter()
                .find(|option| name.eq_ignore_ascii_case(&option.plural))
                .map(|option| (option, true))
        })
        .or_else(|| {
            options
                .iter()
                .find(|option| name.eq_ignore_ascii_case(&option.singular))
                .map(|option| (option, false))
        })
        .or_else(|| user_type_option(name, options))
}

pub(super) fn user_type_option<'a>(
    name: &str,
    options: &'a [ExpressionTypeOption],
) -> Option<(&'a ExpressionTypeOption, bool)> {
    let name = crate::language::strip_indefinite_article(name.trim());
    options.iter().find_map(|option| {
        if option.user_input_patterns.is_empty() {
            return None;
        }
        if name.eq_ignore_ascii_case(&option.plural) {
            return Some((option, true));
        }
        if name.eq_ignore_ascii_case(&option.singular) {
            return Some((option, false));
        }
        option
            .user_input_patterns
            .iter()
            .any(|pattern| matches_user_input(pattern, name))
            .then_some((option, is_plural_user_input(name, option)))
    })
}

pub(super) fn is_plural_user_input(name: &str, option: &ExpressionTypeOption) -> bool {
    if name.eq_ignore_ascii_case(&option.plural) {
        return true;
    }
    if name.eq_ignore_ascii_case(&option.singular) {
        return false;
    }

    let name = name.to_ascii_lowercase();
    let singular = option.singular.to_ascii_lowercase();
    let plural = option.plural.to_ascii_lowercase();

    // User patterns often abbreviate the noun (`num(s)?`) rather than spelling
    // out the documented noun. Infer only the common endings represented by
    // the registered noun; unknown irregular forms stay conservative.
    if singular.ends_with('y') && plural.ends_with("ies") {
        return name.ends_with("ies");
    }
    if (singular.ends_with("fe") || singular.ends_with('f')) && plural.ends_with("ves") {
        return name.ends_with("ves");
    }
    if singular.ends_with("man") && plural.ends_with("men") {
        return name.ends_with("men");
    }
    plural.ends_with('s') && !singular.ends_with('s') && name.ends_with('s')
}

fn matches_user_input(pattern: &str, input: &str) -> bool {
    matches_user_input_pattern(pattern, input)
}

#[cfg(target_arch = "wasm32")]
fn matches_user_input_pattern(pattern: &str, input: &str) -> bool {
    let key = format!("type.user-input-pattern:{pattern}");
    let pattern = format!("(?i:{pattern})");
    crate::nlaocs::skript_parser_addon::catalog_data::language_pattern_matches(
        &key, &pattern, input,
    )
    .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn matches_user_input_pattern(pattern: &str, input: &str) -> bool {
    USER_INPUT_PATTERNS.with(|patterns| {
        let mut patterns = patterns.borrow_mut();
        patterns
            .entry(pattern.to_owned())
            .or_insert_with(|| Regex::new(&format!("(?i)^(?:{pattern})$")).ok())
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(input).unwrap_or(false))
    })
}

#[cfg(test)]
mod tests {
    use super::{is_plural_user_input, type_option, user_type_option};
    use crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption;

    fn option(singular: &str, plural: &str, patterns: &[&str]) -> ExpressionTypeOption {
        ExpressionTypeOption {
            source_record: None,
            definition_id: "type:fixture".to_owned(),
            registration_id: "type:fixture:0".to_owned(),
            addon_name: "fixture".to_owned(),
            addon_version: "1.0.0".to_owned(),
            code_name: "fixture".to_owned(),
            class_name: "fixture.Type".to_owned(),
            parser_class: None,
            type_parse_order: 0,
            before: Vec::new(),
            after: Vec::new(),
            singular: singular.to_owned(),
            plural: plural.to_owned(),
            user_input_patterns: patterns
                .iter()
                .map(|pattern| (*pattern).to_owned())
                .collect(),
            has_parser: true,
            parse_contexts: vec!["DEFAULT".to_owned()],
            has_supplier: false,
            default_expression: None,
        }
    }

    fn named_option(
        code_name: &str,
        class_name: &str,
        singular: &str,
        plural: &str,
        patterns: &[&str],
    ) -> ExpressionTypeOption {
        let mut option = option(singular, plural, patterns);
        option.code_name = code_name.to_owned();
        option.class_name = class_name.to_owned();
        option
    }

    #[test]
    fn registered_noun_patterns_preserve_irregular_plurality() {
        let option = option("person", "people", &["person|people"]);
        let options = [option];
        assert!(type_option("people", &options).is_some_and(|(_, plural)| plural));
        assert!(type_option("person", &options).is_some_and(|(_, plural)| !plural));
    }

    #[test]
    fn names_without_user_patterns_are_not_class_info_literals() {
        let option = option("fixture", "fixtures", &[]);
        assert!(user_type_option("fixture", &[option]).is_none());
    }

    #[test]
    fn class_info_literals_ignore_skript_indefinite_articles() {
        let option = option("item", "items", &["items?"]);
        assert!(user_type_option("an item", &[option]).is_some());
    }

    #[test]
    fn internal_type_lookup_still_accepts_code_names() {
        let option = option("fixture", "fixtures", &[]);
        assert!(type_option("fixture", &[option]).is_some());
    }

    #[test]
    fn code_name_wins_over_an_earlier_types_shared_noun() {
        let options = [
            named_option("double", "java.lang.Double", "number", "numbers", &[]),
            named_option(
                "number",
                "java.lang.Number",
                "number",
                "numbers",
                &["num(ber)?s?"],
            ),
        ];
        let (option, plural) = type_option("number", &options).unwrap();
        assert_eq!(option.class_name, "java.lang.Number");
        assert!(!plural);
    }

    #[test]
    fn user_type_lookup_ignores_nouns_without_user_input_patterns() {
        let options = [
            named_option("double", "java.lang.Double", "number", "numbers", &[]),
            named_option(
                "number",
                "java.lang.Number",
                "number",
                "numbers",
                &["num(ber)?s?"],
            ),
        ];
        let (option, plural) = user_type_option("number", &options).unwrap();
        assert_eq!(option.class_name, "java.lang.Number");
        assert!(!plural);
    }

    #[test]
    fn abbreviated_user_patterns_use_the_registered_noun_ending() {
        let option = option("entity", "entities", &["entit(y|ies)"]);
        assert!(!is_plural_user_input("entity", &option));
        assert!(is_plural_user_input("entities", &option));
    }
}
