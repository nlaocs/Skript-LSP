pub trait Syntax {}
pub trait SkriptHubSyntax: Syntax {
    fn _from_abstract_syntax_list_entry(
        src: &crate::api::types::AbstractAddonSyntaxListEntry,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        Self: Sized;
    fn _get_link(&self) -> String;
}
