#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

/// Legacy parser for the flattened function strings returned by SkriptHub.
///
/// SSG consumers should use the structured `syntaxes::Function` model instead.
pub mod function_pattern;

/// Legacy object model and conversion from API entries.
#[allow(missing_docs)]
pub mod addon_syntax_list;
/// Blocking SkriptHub API client and response DTOs.
#[allow(missing_docs)]
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
