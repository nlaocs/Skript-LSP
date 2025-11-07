/// すべてのsyntax型の基底トレイト
pub trait Syntax {}
/// SkriptHubから入手したsyntaxの型の基底トレイト
/// 後に、プラグインのような形でユーザーが自由に構文を追加できるようにするので、
/// 正式なSyntaxとユーザーが追加したSyntaxを区別するためにこのトレイトを用いる。
pub trait SkriptHubSyntax: Syntax {
    /// SkriptHubのAbstractAddonSyntaxListEntryから変換する
    fn _from_abstract_syntax_list_entry(
        src: &crate::api::types::AbstractAddonSyntaxListEntry,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        Self: Sized;
    /// SkriptHub上のリンクを取得する
    fn _get_link(&self) -> String;
}
