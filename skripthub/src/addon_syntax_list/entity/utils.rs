#![allow(dead_code)]
macro_rules! define_syntax_struct {
    ($name:ident, $parse_func:path, $pattern_type:ty, {
        $($field:ident : $ty:ty = $func:expr),* $(,)?
    }) => {
        #[derive(Debug, Clone, PartialEq)]
        #[allow(dead_code)]
        pub struct $name {
            pub id: i64,
            pub creator: i64,
            pub title: String,
            pub description: Option<std::sync::Arc<str>>,
            pub syntax_pattern: $pattern_type,
            pub raw_syntax_pattern: Vec<String>,
            pub compatible_addon_version: Option<Vec<String>>,
            pub compatible_minecraft_version: Option<std::sync::Arc<str>>,
            pub get_syntax_type_css_class: std::sync::Arc<str>,
            pub required_plugins: Option<Vec<crate::addon_syntax_list::entity::utils::SupportingPlugin>>,
            pub addon: crate::api::types::InternalAddon,
            pub json_id: Option<String>,
            pub created_at: time::OffsetDateTime,
            pub updated_at: time::OffsetDateTime,
            pub keywords: Option<Vec<String>>,
            pub mark_as_removed: bool,
            pub removed_since: Option<String>,
            $(pub $field: $ty),*
        }
        #[allow(dead_code)]
        impl $name {
            pub fn from_abstract_syntax_list_entry(
                src: crate::api::types::AbstractAddonSyntaxListEntry
            ) -> Result<Self, Box<dyn std::error::Error>> {
                let syntax_pattern = {
                    $parse_func(&src.syntax_pattern)?
                };
                Ok(Self {
                    id: src.id,
                    creator: src.creator,
                    title: src.title.clone(),
                    description: if src.description.trim().is_empty() {
                        None
                    } else {
                        Some(src.description.clone())
                    },
                    syntax_pattern,
                    raw_syntax_pattern: src.syntax_pattern.lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    compatible_addon_version: if src.compatible_addon_version.trim().is_empty() {
                        None
                    } else {
                        Some(src.compatible_addon_version.split(',').map(|s| s.trim().to_string()).collect())
                    },
                    compatible_minecraft_version: if src.compatible_minecraft_version.trim().is_empty() {
                        None
                    } else {
                        Some(src.compatible_minecraft_version.clone())
                    },
                    get_syntax_type_css_class: src.get_syntax_type_css_class.clone(),
                    required_plugins: if src.required_plugins.is_empty() {
                        None
                    } else {
                        let rp = &src.required_plugins;
                        let mut r = Vec::with_capacity(rp.len());
                        for plugin in rp {
                            r.push(crate::addon_syntax_list::entity::utils::SupportingPlugin {
                                name: plugin.name.clone(),
                                link: if plugin.link.is_empty() {
                                    None
                                } else {
                                    Some(plugin.link.clone())
                                }
                            });
                        }
                        Some(r)
                    },
                    addon: src.addon.clone(),
                    json_id: src.json_id.as_ref().filter(|s| !s.trim().is_empty()).cloned(),
                    created_at: time::OffsetDateTime::parse(&src.created_at, &time::format_description::well_known::Rfc3339)
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
                    updated_at: time::OffsetDateTime::parse(&src.updated_at, &time::format_description::well_known::Rfc3339)
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
                    keywords: src.keywords.clone().and_then(|s| {
                        if s.trim().is_empty() {
                            None
                        } else {
                            Some(s.split(',').map(|s| s.trim().to_string()).collect())
                        }
                    }),
                    mark_as_removed: src.mark_as_removed,
                    removed_since: src.removed_since.clone(),
                    $($field: $func(&src),)*
                })
            }
            pub fn get_link(&self) -> String {
                format!(
                    "https://skripthub.net/docs/?id={}",
                    self.id
                )
            }
        }
    };
    ($name:ident { $($field:ident : $ty:ty = $func:expr),* $(,)? }) => {
        fn default_syntax_parser(src: &str) -> Result<Vec<syntax_pattern_parser::syntax::ParseResult>, syntax_pattern_parser::syntax::ParseError> {
            let patterns = src.lines();
            let mut parsed_patterns = Vec::new();
            for pattern in patterns {
                let pattern = pattern.trim();
                if pattern.is_empty() {
                    continue;
                }
                parsed_patterns.push(syntax_pattern_parser::syntax::parse(pattern)?);
            }
            Ok(parsed_patterns)
        }
        define_syntax_struct!($name, default_syntax_parser, Vec<syntax_pattern_parser::syntax::ParseResult>, { $($field : $ty = $func),* });
    };
}

pub(crate) use define_syntax_struct;
use std::sync::Arc;

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Entries(pub Vec<EntriesEntry>);
impl std::ops::Deref for Entries {
    type Target = Vec<EntriesEntry>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Entries {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl IntoIterator for Entries {
    type Item = EntriesEntry;
    type IntoIter = std::vec::IntoIter<EntriesEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a Entries {
    type Item = &'a EntriesEntry;
    type IntoIter = std::slice::Iter<'a, EntriesEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Entries {
    pub fn from_json_str(s: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let entries: Entries = serde_json::from_str(s)?;
        Ok(entries)
    }
}
#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntriesEntry {
    pub name: String,
    pub is_required: bool,
    pub is_section: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportingPlugin {
    pub name: Arc<str>,
    pub link: Option<Arc<str>>,
}

#[inline(always)]
pub(crate) fn entries_parser(
    s: &crate::api::types::AbstractAddonSyntaxListEntry,
) -> Option<Entries> {
    let entries = s.entries.clone()?;
    Entries::from_json_str(&entries).ok()
}

#[inline(always)]
pub(crate) fn return_type_parser(
    s: &crate::api::types::AbstractAddonSyntaxListEntry,
) -> Option<String> {
    match &s.return_type {
        Some(return_type) => {
            if return_type.trim().is_empty() {
                None
            } else {
                Some(return_type.clone())
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_creation() {
        let json = r#"[{"name": "timeout", "isRequired": false, "isSection": false, "defaultValue": "null"}, {"name": "follow redirects", "isRequired": false, "isSection": false, "defaultValue": "null"}, {"name": "priority", "isRequired": false, "isSection": false, "defaultValue": "null"}, {"name": "version", "isRequired": false, "isSection": false, "defaultValue": "null"}, {"name": "executor", "isRequired": false, "isSection": true, "defaultValue": "null"}]"#;
        let entries = Entries::from_json_str(json).unwrap();
        assert_eq!(
            entries,
            Entries(vec![
                EntriesEntry {
                    name: "timeout".into(),
                    is_required: false,
                    is_section: false,
                    default_value: Some("null".into()),
                },
                EntriesEntry {
                    name: "follow redirects".into(),
                    is_required: false,
                    is_section: false,
                    default_value: Some("null".into()),
                },
                EntriesEntry {
                    name: "priority".into(),
                    is_required: false,
                    is_section: false,
                    default_value: Some("null".into()),
                },
                EntriesEntry {
                    name: "version".into(),
                    is_required: false,
                    is_section: false,
                    default_value: Some("null".into()),
                },
                EntriesEntry {
                    name: "executor".into(),
                    is_required: false,
                    is_section: true,
                    default_value: Some("null".into()),
                },
            ])
        );
    }
}
