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
    for index in 0..100 {
        let id_key = format!("genders.{index}.id");
        let article_key = format!("genders.{index}.indefinite article");
        if optional_value(&id_key).is_none() && optional_value(&article_key).is_none() {
            break;
        }
        if let Some(stripped) = optional_value(&article_key)
            .as_deref()
            .and_then(|article| strip_prefix_ignore_ascii_case(value, article))
        {
            return stripped;
        }
    }
    for article in ["a", "an"] {
        if let Some(stripped) = strip_prefix_ignore_ascii_case(value, article) {
            return stripped;
        }
    }
    value
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

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    let tail = value.get(prefix.len()..)?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| tail.strip_prefix(' '))
        .flatten()
}

pub(crate) fn clear() {
    VALUES.with(|values| values.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::{is_indefinite_article, strip_indefinite_article};

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
