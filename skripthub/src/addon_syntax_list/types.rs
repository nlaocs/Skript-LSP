/// Marker trait for syntax values in the legacy SkriptHub model.
pub trait Syntax {}
/// Trait implemented by legacy syntax entity types converted from SkriptHub API
/// entries. It provides the conversion and SkriptHub link methods used by the
/// compatibility layer.
pub trait SkriptHubSyntax: Syntax {
    /// SkriptHubのAbstractAddonSyntaxListEntryから変換する
    fn _from_abstract_syntax_list_entry(
        src: &crate::api::types::AbstractAddonSyntaxListEntry,
        plural_rules: &syntax_pattern_parser::syntax::PluralRules,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        Self: Sized;
    /// SkriptHub上のリンクを取得する
    fn _get_link(&self) -> String;
}
