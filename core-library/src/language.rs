use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static VALUES: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

pub(crate) fn value(key: &str, fallback: &str) -> String {
    optional_value(key).unwrap_or_else(|| fallback.to_owned())
}

pub(crate) fn optional_value(key: &str) -> Option<String> {
    VALUES.with(|values| {
        if let Some(value) = values.borrow().get(key).cloned() {
            return value.clone();
        }
        #[cfg(test)]
        let value = None;
        #[cfg(not(test))]
        let value = crate::nlaocs::skript_parser_addon::catalog_data::language_value(key)
            .ok()
            .flatten();
        values.borrow_mut().insert(key.to_owned(), value.clone());
        value
    })
}

pub(crate) fn strip_indefinite_article(value: &str) -> &str {
    parser_wasm::type_language::strip_indefinite_article(value, optional_value)
}

pub(crate) fn is_indefinite_article(value: &str) -> bool {
    for index in 0..100 {
        let id_key = format!("genders.{index}.id");
        let article_key = format!("genders.{index}.indefinite article");
        if optional_value(&id_key).is_none() && optional_value(&article_key).is_none() {
            break;
        }
        if optional_value(&article_key)
            .as_deref()
            .is_some_and(|article| value.eq_ignore_ascii_case(article))
        {
            return true;
        }
    }
    value.eq_ignore_ascii_case("a") || value.eq_ignore_ascii_case("an")
}

pub(crate) fn clear() {
    VALUES.with(|values| values.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::{is_indefinite_article, strip_indefinite_article};

    #[test]
    fn shared_type_preprocessing_keeps_localized_order_and_one_strip() {
        let values = std::collections::HashMap::from([
            ("genders.0.id", "article"),
            ("genders.0.indefinite article", "une"),
            // A gap ends the scan, just as in the existing Core parser.
            ("genders.2.indefinite article", "ignored"),
        ]);
        for (input, expected) in [
            ("UNE widget", "widget"),
            ("a a widget", "a widget"),
            ("an item", "item"),
            ("une  widget", " widget"),
            ("ignored widget", "ignored widget"),
            ("another item", "another item"),
            ("界widget", "界widget"),
        ] {
            assert_eq!(
                parser_wasm::type_language::strip_indefinite_article(input, |key| {
                    values.get(key).map(|v| (*v).to_owned())
                }),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn strips_the_default_skript_indefinite_articles() {
        assert_eq!(strip_indefinite_article("a player"), "player");
        assert_eq!(strip_indefinite_article("AN item"), "item");
        assert_eq!(strip_indefinite_article("another item"), "another item");
        assert!(is_indefinite_article("a"));
        assert!(is_indefinite_article("AN"));
        assert!(!is_indefinite_article("the"));
    }
}
