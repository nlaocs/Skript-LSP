use std::sync::{Arc, LazyLock};

pub(crate) fn intern_arc_str_with_empty<'de, D>(deserializer: D) -> Result<Arc<str>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::de::{Deserialize, IntoDeserializer};
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        static EMPTY: LazyLock<Arc<str>> = LazyLock::new(|| Arc::<str>::from(""));
        return Ok(EMPTY.clone());
    }
    serde_intern::intern_arc_str(IntoDeserializer::into_deserializer(s))
}
