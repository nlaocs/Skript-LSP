//! Shared ClassInfo input preprocessing; host and guest use identical language rules.

/// Strips at most one localized indefinite article, then the standard a/an
/// fallback. The result always borrows the original input. The caller supplies
/// the same immutable language values used by its parser context.
pub fn strip_indefinite_article(
    value: &str,
    mut language_value: impl FnMut(&str) -> Option<String>,
) -> &str {
    for index in 0..100 {
        let id_key = format!("genders.{index}.id");
        let article_key = format!("genders.{index}.indefinite article");
        if language_value(&id_key).is_none() && language_value(&article_key).is_none() {
            break;
        }
        if let Some(stripped) = language_value(&article_key)
            .as_deref()
            .and_then(|article| strip_prefix(value, article))
        {
            return stripped;
        }
    }
    for article in ["a", "an"] {
        if let Some(stripped) = strip_prefix(value, article) {
            return stripped;
        }
    }
    value
}

fn strip_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    let tail = value.get(prefix.len()..)?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| tail.strip_prefix(' '))
        .flatten()
}
