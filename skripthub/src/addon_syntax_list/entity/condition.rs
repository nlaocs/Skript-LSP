//! Legacy SkriptHub condition entity.

use crate::addon_syntax_list::entity::utils::define_syntax_struct;

define_syntax_struct!(Condition {});

#[cfg(test)]
mod tests {
    use super::Condition;
    use crate::addon_syntax_list::entity::utils::SupportingPlugin;
    use crate::api::types::AbstractAddonSyntaxListEntry;
    use std::sync::Arc;

    #[test]
    fn condition_creation() {
        let json = r#"{
        "id": 267,
        "creator": 4,
        "title": "File Exists",
        "description": "Checks if the file exists or doesn't exist",
        "syntax_pattern": "(script|program|app[lication]|file|dir[ectory]) %string% exists\n(script|program|app[lication]|file|dir[ectory]) %string% does(n't| not) exist",
        "compatible_addon_version": "1.0.0",
        "compatible_minecraft_version": "1.12.2",
        "syntax_type": "condition",
        "get_syntax_type_css_class": "syntax_title_box_condition",
        "required_plugins": [
            {
                "name": "Protocollib",
                "link": "https://www.spigotmc.org/resources/protocollib.1997/"
            },
            {
                "name": "Holographic Displays",
                "link": "https://dev.bukkit.org/projects/holographic-displays"
            }
        ],
        "addon": {
            "name": "skUtilities",
            "link_to_addon": "https://forums.skunity.com/resources/skutilities.26/",
            "usage_score": 32.1
        },
        "type_usage": "",
        "return_type": null,
        "event_values": null,
        "json_id": null,
        "event_cancellable": false,
        "created_at": "2017-09-27T19:45:27.313208Z",
        "updated_at": "2017-09-27T19:45:27.313236Z",
        "entries": null,
        "keywords": "amount",
        "mark_as_removed": false,
        "removed_since": null
    }"#;
        let parsed: AbstractAddonSyntaxListEntry = serde_json::from_str(json).unwrap();
        let condition =
            Condition::from_abstract_syntax_list_entry(&parsed, crate::test_plural_rules())
                .unwrap();
        assert_eq!(condition.id, 267);
        assert_eq!(condition.creator, 4);
        assert_eq!(condition.title, "File Exists");
        assert_eq!(
            condition.description,
            Some(Arc::<str>::from(
                "Checks if the file exists or doesn't exist"
            ))
        );
        // todo パース結果の確認をするとsyntax_patternのテストも担ってしまうので今は省略
        assert!(!condition.syntax_pattern.is_empty());
        assert_eq!(
            condition.raw_syntax_pattern,
            vec![
                "(script|program|app[lication]|file|dir[ectory]) %string% exists",
                "(script|program|app[lication]|file|dir[ectory]) %string% does(n't| not) exist"
            ]
        );
        assert_eq!(
            condition.compatible_addon_version,
            Some(vec!["1.0.0".to_string()])
        );
        assert_eq!(
            condition.compatible_minecraft_version,
            Some(Arc::<str>::from("1.12.2"))
        );
        assert_eq!(
            condition.get_syntax_type_css_class,
            Arc::<str>::from("syntax_title_box_condition")
        );
        assert_eq!(
            condition.required_plugins,
            Some(vec![
                SupportingPlugin {
                    name: Arc::<str>::from("Protocollib"),
                    link: Some(Arc::<str>::from(
                        "https://www.spigotmc.org/resources/protocollib.1997/"
                    )),
                },
                SupportingPlugin {
                    name: Arc::<str>::from("Holographic Displays"),
                    link: Some(Arc::<str>::from(
                        "https://dev.bukkit.org/projects/holographic-displays"
                    )),
                },
            ])
        );
        assert_eq!(condition.addon.name, Arc::<str>::from("skUtilities"));
        assert_eq!(
            condition.addon.link_to_addon,
            Arc::<str>::from("https://forums.skunity.com/resources/skutilities.26/")
        );
        assert_eq!(condition.addon.usage_score, 32.1);
        assert_eq!(condition.json_id, None);
        assert_eq!(
            condition.created_at.to_string(),
            "2017-09-27 19:45:27.313208 +00:00:00"
        );
        assert_eq!(
            condition.updated_at.to_string(),
            "2017-09-27 19:45:27.313236 +00:00:00"
        );
        assert_eq!(condition.keywords, Some(vec!["amount".to_string()]));
        assert!(!condition.mark_as_removed);
        assert_eq!(condition.removed_since, None);
    }
}
