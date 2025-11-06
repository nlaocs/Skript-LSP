#![allow(dead_code)]
use crate::api::internal_utils::intern_arc_str_with_empty;
use serde_intern::intern_arc_str;
use std::sync::Arc;

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub struct AbstractAddonSyntaxList(Vec<AbstractAddonSyntaxListEntry>);
impl std::ops::Deref for AbstractAddonSyntaxList {
    type Target = Vec<AbstractAddonSyntaxListEntry>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for AbstractAddonSyntaxList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl IntoIterator for AbstractAddonSyntaxList {
    type Item = AbstractAddonSyntaxListEntry;
    type IntoIter = std::vec::IntoIter<AbstractAddonSyntaxListEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a AbstractAddonSyntaxList {
    type Item = &'a AbstractAddonSyntaxListEntry;
    type IntoIter = std::slice::Iter<'a, AbstractAddonSyntaxListEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SyntaxType {
    Event,
    Condition,
    Effect,
    Expression,
    Type,
    Function,
    Section,
    Structure,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SupportingPlugin {
    #[serde(deserialize_with = "intern_arc_str")]
    pub name: Arc<str>, // required [1..80]
    #[serde(deserialize_with = "intern_arc_str")]
    pub link: Arc<str>, // <=200
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub struct InternalAddon {
    #[serde(deserialize_with = "intern_arc_str")]
    pub name: Arc<str>, // required [1..40]
    #[serde(deserialize_with = "intern_arc_str")]
    pub link_to_addon: Arc<str>, // <uri> required [1..200]
    pub usage_score: f64,
}
#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub struct AbstractAddonSyntaxListEntry {
    pub id: i64,
    pub creator: i64,
    pub title: String, // required [1..100]
    #[serde(deserialize_with = "intern_arc_str_with_empty")]
    pub description: Arc<str>, // <=8000
    #[serde(deserialize_with = "intern_arc_str")]
    pub syntax_pattern: Arc<str>, // required [1..3500]
    #[serde(deserialize_with = "intern_arc_str")]
    pub compatible_addon_version: Arc<str>, // <=200
    #[serde(deserialize_with = "intern_arc_str")]
    pub compatible_minecraft_version: Arc<str>, // <=200
    pub syntax_type: SyntaxType,
    #[serde(deserialize_with = "intern_arc_str")]
    pub get_syntax_type_css_class: Arc<str>, // 使うのか...?
    pub required_plugins: Vec<SupportingPlugin>,
    pub addon: InternalAddon,
    pub type_usage: Option<String>,   // <=2000
    pub return_type: Option<String>,  // <=200
    pub event_values: Option<String>, // <=1000
    pub json_id: Option<String>,      // <=200
    pub event_cancellable: bool,
    pub created_at: String,       // <date-time>
    pub updated_at: String,       // <date-time>
    pub entries: Option<String>,  // <=3500
    pub keywords: Option<String>, // <=200
    pub mark_as_removed: bool,
    pub removed_since: Option<String>, // <=100
}
