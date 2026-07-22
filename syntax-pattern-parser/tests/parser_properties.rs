mod support;

use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestCaseResult};
use std::sync::LazyLock;
use syntax_pattern_parser::syntax::PluralRules;

static PLURAL_RULES: LazyLock<PluralRules> = LazyLock::new(|| {
    PluralRules::from_json(include_str!("data/PluralRules-2.15.4.json"))
        .expect("generated PluralRules-2.15.4.json fixture must be valid")
});

fn arbitrary_utf8() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..257)
        .prop_map(|characters| characters.into_iter().collect())
}

fn delimiter_heavy_utf8() -> impl Strategy<Value = String> {
    let delimiters = prop::sample::select(vec![
        '(', ')', '[', ']', '<', '>', '%', '|', '\\', ':', '¦', '@', '-', '*', '~', '/',
    ]);

    prop::collection::vec(prop_oneof![8 => delimiters, 2 => any::<char>()], 0..513)
        .prop_map(|characters| characters.into_iter().collect())
}

fn assert_invariants(pattern: &str) -> TestCaseResult {
    support::validate_parse_invariants(pattern, &PLURAL_RULES).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_utf8_is_deterministic_and_has_valid_spans(pattern in arbitrary_utf8()) {
        assert_invariants(&pattern)?;
    }

    #[test]
    fn delimiter_heavy_utf8_is_deterministic_and_has_valid_spans(
        pattern in delimiter_heavy_utf8()
    ) {
        assert_invariants(&pattern)?;
    }
}

#[test]
fn adversarial_regressions_complete_without_panicking() {
    let cases = [
        format!("{}value{}", "(".repeat(128), ")".repeat(127)),
        format!("{}value{}", "[".repeat(128), "]".repeat(127)),
        "|".repeat(4096),
        "%".repeat(4096),
        "\\".repeat(4096),
        "構文".repeat(8192),
        "tag:value ".repeat(1024),
        "1¦marked|".repeat(1024),
        "末尾%".to_string(),
        "末尾<".to_string(),
        "末尾\\".to_string(),
        "[(group]".to_string(),
        "([option)".to_string(),
    ];

    for pattern in cases {
        support::validate_parse_invariants(&pattern, &PLURAL_RULES)
            .unwrap_or_else(|error| panic!("{error}\npattern={pattern:?}"));
    }
}
