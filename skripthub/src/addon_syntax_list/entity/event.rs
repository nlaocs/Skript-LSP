use crate::addon_syntax_list::entity::utils::define_syntax_struct;

#[inline(always)]
fn event_values_parser(s: &crate::api::types::AbstractAddonSyntaxListEntry) -> Option<Vec<String>> {
    s.event_values
        .as_ref()
        .map(|s| {
            s.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s.to_lowercase() != "none") // "none"は無視する。詳しくはid2520のevent_valuesを参照 (https://skripthub.net/docs/?id=2520)
                .collect::<Vec<String>>()
        })
        .filter(|s| !s.is_empty())
}

#[inline(always)]
fn event_cancellable_parser(s: &crate::api::types::AbstractAddonSyntaxListEntry) -> bool {
    s.event_cancellable
}

define_syntax_struct!(Event {
    event_values: Option<Vec<String>> = event_values_parser,
    event_cancellable: bool = event_cancellable_parser,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::AbstractAddonSyntaxListEntry;

    #[test]
    fn event_creation() {
        // Eventでの独自の実装以外はConditionと同じ実装をしているため、省略
        let json = r#"{
        "id": 1097,
        "creator": 6,
        "title": "At Time",
        "description": "An event that occurs at a given minecraft time in every world or only in specific worlds.",
        "syntax_pattern": "[on] at %time% [in %worlds%]",
        "compatible_addon_version": "1.3.4",
        "compatible_minecraft_version": "",
        "syntax_type": "event",
        "get_syntax_type_css_class": "syntax_title_box_event",
        "required_plugins": [],
        "addon": {
            "name": "Skript",
            "link_to_addon": "https://github.com/SkriptLang/Skript",
            "usage_score": 1480.1
        },
        "type_usage": "",
        "return_type": "",
        "event_values": "event-world",
        "json_id": "at_time",
        "event_cancellable": false,
        "created_at": "2017-10-04T00:46:13.408501Z",
        "updated_at": "2025-04-02T05:16:38.933553Z",
        "entries": "",
        "keywords": null,
        "mark_as_removed": false,
        "removed_since": null
    }
        "#;
        let parsed: AbstractAddonSyntaxListEntry = serde_json::from_str(json).unwrap();
        let event = Event::from_abstract_syntax_list_entry(&parsed).unwrap();
        assert_eq!(event.event_values, Some(vec!["event-world".to_string()]));
        assert!(!event.event_cancellable);
    }
}
