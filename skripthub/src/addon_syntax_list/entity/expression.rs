use crate::addon_syntax_list::entity::internal_utils::define_syntax_struct;

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

define_syntax_struct!(Expression {
    return_type: Option<String> = return_type_parser,
});

#[cfg(test)]
mod tests {
    use crate::addon_syntax_list::entity::expression::Expression;
    use crate::api::types::AbstractAddonSyntaxListEntry;

    #[test]
    fn expression_creation() {
        // Expressionでの独自の実装以外はConditionと同じ実装をしているため、省略
        let json = r#"{
        "id": 12251,
        "creator": 2577,
        "title": "Wandering Trader - Drink State",
        "description": "Gets/sets the drink state of a wandering trader.",
        "syntax_pattern": "[the] [wandering[ ]trader] [can] drink (milk|[a] potion) [(state|mode)] of %entities%\n%entities%'[s] [wandering[ ]trader] [can] drink (milk|[a] potion) [(state|mode)]",
        "compatible_addon_version": "2.8",
        "compatible_minecraft_version": "",
        "syntax_type": "expression",
        "get_syntax_type_css_class": "syntax_title_box_expression",
        "required_plugins": [],
        "addon": {
            "name": "Skuishy",
            "link_to_addon": "https://github.com/Fusezion/Skuishy",
            "usage_score": 28.2
        },
        "type_usage": "",
        "return_type": "Boolean",
        "event_values": "",
        "json_id": "ExprWanderingTraderDrink",
        "event_cancellable": false,
        "created_at": "2024-10-13T22:57:02.638351Z",
        "updated_at": "2024-10-13T22:57:02.638366Z",
        "entries": "",
        "keywords": null,
        "mark_as_removed": false,
        "removed_since": null
    }
    "#;
        let parsed: AbstractAddonSyntaxListEntry = serde_json::from_str(json).unwrap();
        let expression = Expression::from_abstract_syntax_list_entry(parsed).unwrap();
        assert_eq!(expression.return_type, Some("Boolean".to_string()));
    }
}
