//! Legacy SkriptHub section entity and entry metadata conversion.

use crate::addon_syntax_list::entity::utils::{Entries, define_syntax_struct, entries_parser};

define_syntax_struct!(Section {
    entries: Option<Entries> = entries_parser,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addon_syntax_list::entity::utils::{Entries, EntriesEntry};
    use crate::api::types::AbstractAddonSyntaxListEntry;

    #[test]
    fn section_creation() {
        // Sectionでの独自の実装以外はConditionと同じ実装をしているため、省略
        let json = r#"
        {
        "id": 12286,
        "creator": 2577,
        "title": "HTTP Client Builder",
        "description": "Builds a HTTP client.",
        "syntax_pattern": "http client [builder] stored in %object%",
        "compatible_addon_version": "1.0",
        "compatible_minecraft_version": "",
        "syntax_type": "section",
        "get_syntax_type_css_class": "syntax_title_box_section",
        "required_plugins": [],
        "addon": {
            "name": "SkHttp",
            "link_to_addon": "https://github.com/aabssmc/SkHttp",
            "usage_score": 18.5
        },
        "type_usage": "",
        "return_type": "",
        "event_values": "",
        "json_id": "SecHttpClientBuilder",
        "event_cancellable": false,
        "created_at": "2024-10-14T04:04:55.737362Z",
        "updated_at": "2024-10-14T04:04:55.737377Z",
        "entries": "[{\"name\": \"timeout\", \"isRequired\": false, \"isSection\": false, \"defaultValue\": \"null\"}, {\"name\": \"follow redirects\", \"isRequired\": false, \"isSection\": false, \"defaultValue\": \"null\"}, {\"name\": \"priority\", \"isRequired\": false, \"isSection\": false, \"defaultValue\": \"null\"}, {\"name\": \"version\", \"isRequired\": false, \"isSection\": false, \"defaultValue\": \"null\"}, {\"name\": \"executor\", \"isRequired\": false, \"isSection\": true, \"defaultValue\": \"null\"}]",
        "keywords": null,
        "mark_as_removed": false,
        "removed_since": null
    }
        "#;
        let parsed: AbstractAddonSyntaxListEntry = serde_json::from_str(json).unwrap();
        let section =
            Section::from_abstract_syntax_list_entry(&parsed, crate::test_plural_rules()).unwrap();
        assert_eq!(
            section.entries,
            Some(Entries(vec![
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
            ],))
        );
    }
}
