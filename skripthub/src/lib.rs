pub mod addon_syntax_list;
pub mod api;
#[cfg(test)]
pub(crate) fn test_plural_rules() -> &'static syntax_pattern_parser::syntax::PluralRules {
    static RULES: std::sync::LazyLock<syntax_pattern_parser::syntax::PluralRules> =
        std::sync::LazyLock::new(|| {
            syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(
                "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
            ))
            .expect("generated PluralRules-2.15.4.json fixture must be valid")
        });
    &RULES
}
