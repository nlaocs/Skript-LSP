//! Legacy SkriptHub flattened-function entity and return-type conversion.

use crate::addon_syntax_list::entity::utils::{define_syntax_struct, return_type_parser};

fn parse_function_pattern(
    source: &str,
    _plural_rules: &syntax_pattern_parser::syntax::PluralRules,
) -> Result<crate::function_pattern::FnParseResult, crate::function_pattern::FnParseError> {
    crate::function_pattern::parse(source)
}

define_syntax_struct!(Function, parse_function_pattern, crate::function_pattern::FnParseResult, {
    return_type: Option<String> = return_type_parser,
});

#[cfg(test)]
mod tests {
    use crate::addon_syntax_list::entity::function::Function;
    use crate::api::types::AbstractAddonSyntaxListEntry;

    #[test]
    fn function_creation() {
        let json = r#"{
        "id": 2113,
        "creator": 1,
        "title": "ceil",
        "description": "Rounds a number up, i.e. returns the closest integer larger than or equal to the argument.",
        "syntax_pattern": "ceil(n: number = 1)",
        "compatible_addon_version": "2.2",
        "compatible_minecraft_version": "",
        "syntax_type": "function",
        "get_syntax_type_css_class": "syntax_title_box_function",
        "required_plugins": [],
        "addon": {
            "name": "Skript",
            "link_to_addon": "https://github.com/SkriptLang/Skript",
            "usage_score": 1480.1
        },
        "type_usage": "",
        "return_type": "long",
        "event_values": "",
        "json_id": "function_ceil",
        "event_cancellable": false,
        "created_at": "2018-01-17T16:35:21.110804Z",
        "updated_at": "2021-11-16T08:55:42.898247Z",
        "entries": null,
        "keywords": null,
        "mark_as_removed": false,
        "removed_since": null
    }"#;
        let parsed: AbstractAddonSyntaxListEntry = serde_json::from_str(json).unwrap();
        let function =
            Function::from_abstract_syntax_list_entry(&parsed, crate::test_plural_rules()).unwrap();
        assert_eq!(function.syntax_pattern.inner.name, "ceil");
        assert_eq!(function.syntax_pattern.inner.args[0].arg_type, "number");
        assert_eq!(function.syntax_pattern.inner.args[0].name, "n");
        assert_eq!(
            function.syntax_pattern.inner.args[0].default_expression,
            Some("1".to_string())
        );
        assert_eq!(function.return_type.as_deref(), Some("long"));
    }
}
