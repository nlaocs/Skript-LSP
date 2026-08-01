//! Server-specific singular/plural conversion used while parsing type expressions.
//!
//! Rules are loaded from SSG output so addon overrides and Skript-version behavior
//! remain part of the snapshot rather than hardcoded LSP behavior.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Skript algorithm used to decide whether a word is already singular.
pub enum PluralAlgorithm {
    /// Older releases apply the first suffix rule that matches.
    LegacyFirstMatch,
    /// Newer releases first protect words that already look singular.
    SingularAware,
    /// Generator could not identify the runtime algorithm; rejected by this crate.
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Registry that contributed a plural conversion rule.
pub enum PluralRuleOrigin {
    /// Rule shipped by Skript itself.
    BuiltIn,
    /// Rule registered at runtime by an addon.
    Override,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
/// Addon identity recorded for a plural rule.
pub struct PluralRuleAddon {
    name: String,
    version: String,
}

impl PluralRuleAddon {
    /// Plugin name reported by the server.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Plugin version reported by the server.
    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
/// One ordered suffix replacement from generated `PluralRules.json`.
pub struct PluralRule {
    rule_order: usize,
    singular: String,
    plural: String,
    complete_word: Option<bool>,
    origin: PluralRuleOrigin,
    override_registration_order: Option<usize>,
    addon: PluralRuleAddon,
}

impl PluralRule {
    /// Zero-based order in which the runtime considers this rule.
    pub fn rule_order(&self) -> usize {
        self.rule_order
    }

    /// Singular suffix, or an empty string for the fallback rule.
    pub fn singular(&self) -> &str {
        &self.singular
    }

    /// Plural suffix replacing [`Self::singular`].
    pub fn plural(&self) -> &str {
        &self.plural
    }

    /// Whether this rule must match the complete word.
    ///
    /// `None` is valid only for the legacy algorithm.
    pub fn complete_word(&self) -> Option<bool> {
        self.complete_word
    }

    /// Returns whether Skript or an addon registered the rule.
    pub fn origin(&self) -> PluralRuleOrigin {
        self.origin
    }

    /// Addon override order, present only for override rules.
    pub fn override_registration_order(&self) -> Option<usize> {
        self.override_registration_order
    }

    /// Returns the owner recorded by the generator.
    pub fn addon(&self) -> &PluralRuleAddon {
        &self.addon
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Validated ordered plural rules for one generated server snapshot.
///
/// Skript applies these suffix replacements when type placeholders are parsed.
/// The order and algorithm differ across Skript versions, and addons may
/// register overrides, so rules must be loaded from the same SSG snapshot as
/// the syntax catalog they accompany.
///
/// # Examples
///
/// ~~~
/// use syntax_pattern_parser::syntax::{PluralAlgorithm, PluralRules};
///
/// let json = r#"{
///     "algorithm": "singular-aware",
///     "pluralOverrideSupported": false,
///     "rules": [
///         {
///             "ruleOrder": 0,
///             "singular": "person",
///             "plural": "people",
///             "completeWord": true,
///             "origin": "built-in",
///             "addon": { "name": "Skript", "version": "example" }
///         },
///         {
///             "ruleOrder": 1,
///             "singular": "",
///             "plural": "s",
///             "completeWord": false,
///             "origin": "built-in",
///             "addon": { "name": "Skript", "version": "example" }
///         }
///     ]
/// }"#;
/// let rules = PluralRules::from_json(json)?;
///
/// assert_eq!(rules.algorithm(), PluralAlgorithm::SingularAware);
/// assert_eq!(rules.to_singular("people"), ("person".to_owned(), true));
/// assert_eq!(rules.to_singular("person"), ("person".to_owned(), false));
/// assert_eq!(rules.to_plural("message"), "messages");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ~~~
pub struct PluralRules {
    algorithm: PluralAlgorithm,
    plural_override_supported: bool,
    rules: Vec<PluralRule>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPluralRules {
    algorithm: PluralAlgorithm,
    plural_override_supported: bool,
    rules: Vec<PluralRule>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Invariant violation in generated `PluralRules.json`.
#[allow(missing_docs)] // Variant messages precisely describe the rejected invariant.
pub enum PluralRulesError {
    #[error("PluralRules.json uses the unresolved algorithm")]
    UnresolvedAlgorithm,
    #[error("PluralRules.json must contain at least one rule")]
    EmptyRules,
    #[error("PluralRules.json ruleOrder must be contiguous: expected {expected}, found {actual}")]
    InvalidRuleOrder { expected: usize, actual: usize },
    #[error("rule {rule_order} must contain completeWord for the singular-aware algorithm")]
    MissingCompleteWord { rule_order: usize },
    #[error("rule {rule_order} must omit completeWord for the legacy-first-match algorithm")]
    UnexpectedCompleteWord { rule_order: usize },
    #[error("rule {rule_order} has a blank addon name or version")]
    InvalidAddon { rule_order: usize },
    #[error("override rule {rule_order} is present while pluralOverrideSupported is false")]
    UnsupportedOverride { rule_order: usize },
    #[error("override rule {rule_order} appears after a built-in rule")]
    OverrideAfterBuiltIn { rule_order: usize },
    #[error("override rule {rule_order} is missing overrideRegistrationOrder")]
    MissingOverrideRegistrationOrder { rule_order: usize },
    #[error("built-in rule {rule_order} must omit overrideRegistrationOrder")]
    UnexpectedOverrideRegistrationOrder { rule_order: usize },
    #[error("overrideRegistrationOrder must be contiguous: expected {expected}, found {actual}")]
    InvalidOverrideRegistrationOrder { expected: usize, actual: usize },
    #[error("the final plural rule must be the built-in, non-complete fallback rule")]
    InvalidFallbackRule,
}

impl TryFrom<RawPluralRules> for PluralRules {
    type Error = PluralRulesError;

    fn try_from(raw: RawPluralRules) -> Result<Self, Self::Error> {
        if raw.algorithm == PluralAlgorithm::Unresolved {
            return Err(PluralRulesError::UnresolvedAlgorithm);
        }
        if raw.rules.is_empty() {
            return Err(PluralRulesError::EmptyRules);
        }

        let mut built_in_reached = false;
        let mut override_orders = Vec::new();

        for (expected_order, rule) in raw.rules.iter().enumerate() {
            if rule.rule_order != expected_order {
                return Err(PluralRulesError::InvalidRuleOrder {
                    expected: expected_order,
                    actual: rule.rule_order,
                });
            }

            match raw.algorithm {
                PluralAlgorithm::SingularAware if rule.complete_word.is_none() => {
                    return Err(PluralRulesError::MissingCompleteWord {
                        rule_order: rule.rule_order,
                    });
                }
                PluralAlgorithm::LegacyFirstMatch if rule.complete_word.is_some() => {
                    return Err(PluralRulesError::UnexpectedCompleteWord {
                        rule_order: rule.rule_order,
                    });
                }
                _ => {}
            }

            if rule.addon.name.trim().is_empty() || rule.addon.version.trim().is_empty() {
                return Err(PluralRulesError::InvalidAddon {
                    rule_order: rule.rule_order,
                });
            }

            match rule.origin {
                PluralRuleOrigin::Override => {
                    if !raw.plural_override_supported {
                        return Err(PluralRulesError::UnsupportedOverride {
                            rule_order: rule.rule_order,
                        });
                    }
                    if built_in_reached {
                        return Err(PluralRulesError::OverrideAfterBuiltIn {
                            rule_order: rule.rule_order,
                        });
                    }
                    override_orders.push(rule.override_registration_order.ok_or(
                        PluralRulesError::MissingOverrideRegistrationOrder {
                            rule_order: rule.rule_order,
                        },
                    )?);
                }
                PluralRuleOrigin::BuiltIn => {
                    built_in_reached = true;
                    if rule.override_registration_order.is_some() {
                        return Err(PluralRulesError::UnexpectedOverrideRegistrationOrder {
                            rule_order: rule.rule_order,
                        });
                    }
                }
            }
        }

        override_orders.sort_unstable();
        for (expected, actual) in override_orders.into_iter().enumerate() {
            if expected != actual {
                return Err(PluralRulesError::InvalidOverrideRegistrationOrder {
                    expected,
                    actual,
                });
            }
        }

        let fallback = raw.rules.last().expect("rules were checked as non-empty");
        if fallback.origin != PluralRuleOrigin::BuiltIn
            || !fallback.singular.is_empty()
            || fallback.complete_word == Some(true)
        {
            return Err(PluralRulesError::InvalidFallbackRule);
        }

        Ok(Self {
            algorithm: raw.algorithm,
            plural_override_supported: raw.plural_override_supported,
            rules: raw.rules,
        })
    }
}

impl<'de> Deserialize<'de> for PluralRules {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPluralRules::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl PluralRules {
    /// Deserializes and validates generated `PluralRules.json`.
    ///
    /// Validation covers contiguous runtime order, version-specific fields,
    /// addon override ownership, and the required final fallback rule. A
    /// successful result is therefore ready for [crate::syntax::parse].
    ///
    /// # Errors
    ///
    /// Returns a JSON error whose message contains the violated
    /// [PluralRulesError] invariant when the serialized rules are malformed.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Returns the runtime algorithm captured by SSG.
    pub fn algorithm(&self) -> PluralAlgorithm {
        self.algorithm
    }

    /// Returns whether this Skript version exposes addon plural overrides.
    pub fn plural_override_supported(&self) -> bool {
        self.plural_override_supported
    }

    /// Returns rules in exact runtime evaluation order.
    pub fn rules(&self) -> &[PluralRule] {
        &self.rules
    }

    /// Converts a type word to its singular code name.
    ///
    /// The boolean reports whether a plural spelling was recognized.
    pub fn to_singular(&self, word: &str) -> (String, bool) {
        if word.is_empty() {
            return (String::new(), false);
        }

        if self.algorithm == PluralAlgorithm::SingularAware && self.could_be_singular(word) {
            return (word.to_string(), false);
        }

        for rule in &self.rules {
            if rule.complete_word == Some(true) && word.len() != rule.plural.len() {
                continue;
            }
            if let Some(stem) = word.strip_suffix(&rule.plural) {
                return (format!("{stem}{}", rule.singular), true);
            }

            let uppercase_plural = rule.plural.to_uppercase();
            if let Some(stem) = word.strip_suffix(&uppercase_plural) {
                return (format!("{stem}{}", rule.singular.to_uppercase()), true);
            }
        }

        (word.to_string(), false)
    }

    /// Converts a singular type code name to its runtime plural spelling.
    pub fn to_plural(&self, word: &str) -> String {
        for rule in &self.rules {
            if rule.complete_word == Some(true) && word.len() != rule.singular.len() {
                continue;
            }
            if let Some(stem) = word.strip_suffix(&rule.singular) {
                return format!("{stem}{}", rule.plural);
            }
        }

        unreachable!("validated plural rules always contain a fallback rule")
    }

    fn could_be_singular(&self, word: &str) -> bool {
        let lowercase = word.to_lowercase();
        self.rules.iter().any(|rule| {
            !rule.singular.trim().is_empty()
                && (rule.complete_word != Some(true) || word.len() == rule.singular.len())
                && (word.ends_with(&rule.singular) || lowercase.ends_with(&rule.singular))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated_rules() -> PluralRules {
        PluralRules::from_json(include_str!("../tests/data/PluralRules-2.15.4.json")).unwrap()
    }

    fn generated_legacy_rules() -> PluralRules {
        PluralRules::from_json(include_str!("../tests/data/PluralRules-2.6.4.json")).unwrap()
    }

    #[test]
    fn reads_generated_ssg_rules_and_applies_addon_override() {
        let rules = generated_rules();

        assert_eq!(rules.algorithm(), PluralAlgorithm::SingularAware);
        assert!(rules.plural_override_supported());
        assert_eq!(
            rules.to_singular("dummyfixturepeople"),
            ("dummyfixtureperson".to_string(), true)
        );
        assert_eq!(rules.to_plural("dummyfixtureperson"), "dummyfixturepeople");
        assert_eq!(rules.rules()[0].origin(), PluralRuleOrigin::Override);
        assert_eq!(rules.rules()[0].addon().name(), "SkriptDummyAddon");
    }

    #[test]
    fn matches_skript_plural_regressions() {
        let rules = generated_rules();
        let pairs = [
            ("house", "houses"),
            ("cookie", "cookies"),
            ("creeper", "creepers"),
            ("cactus", "cacti"),
            ("rose", "roses"),
            ("dye", "dyes"),
            ("name", "names"),
            ("ingot", "ingots"),
            ("derp", "derps"),
            ("choir", "choirs"),
            ("man", "men"),
            ("child", "children"),
            ("hoe", "hoes"),
            ("toe", "toes"),
            ("hero", "heroes"),
            ("kidney", "kidneys"),
            ("anatomy", "anatomies"),
            ("axe", "axes"),
            ("knife", "knives"),
            ("elf", "elves"),
            ("shelf", "shelves"),
            ("self", "selves"),
            ("gui", "guis"),
        ];

        for (singular, plural) in pairs {
            assert_eq!(rules.to_plural(singular), plural);
            assert_eq!(rules.to_singular(plural), (singular.to_string(), true));
        }

        assert_eq!(rules.to_plural("sheep"), "sheep");
        assert_eq!(rules.to_singular("sheep"), ("sheep".to_string(), false));
    }

    #[test]
    fn reads_generated_legacy_rules_and_distinguishes_algorithms() {
        let legacy = generated_legacy_rules();

        assert_eq!(legacy.algorithm(), PluralAlgorithm::LegacyFirstMatch);
        assert!(!legacy.plural_override_supported());
        assert!(
            legacy
                .rules()
                .iter()
                .all(|rule| rule.complete_word().is_none())
        );
        assert_eq!(legacy.to_singular("cactus"), ("cactu".to_string(), true));
        assert_eq!(
            generated_rules().to_singular("cactus"),
            ("cactus".to_string(), false)
        );
    }

    #[test]
    fn rejects_unresolved_and_malformed_rule_sets() {
        let unresolved = r#"{
            "algorithm": "unresolved",
            "pluralOverrideSupported": false,
            "rules": []
        }"#;
        assert!(PluralRules::from_json(unresolved).is_err());

        let invalid_order = r#"{
            "algorithm": "singular-aware",
            "pluralOverrideSupported": false,
            "rules": [{
                "ruleOrder": 1,
                "singular": "",
                "plural": "s",
                "completeWord": false,
                "origin": "built-in",
                "addon": {"name": "Skript", "version": "2.15.4"}
            }]
        }"#;
        assert!(PluralRules::from_json(invalid_order).is_err());
    }
}
