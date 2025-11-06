use crate::addon_syntax_list::entity::utils::define_syntax_struct;

fn type_usage_parser(_s: &crate::api::types::AbstractAddonSyntaxListEntry) -> Vec<String> {
    //s.type_usage.clone().unwrap_or_default()
    vec![] // todo
}

define_syntax_struct!(Type {
    type_usage: Vec<String> = type_usage_parser,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::AbstractAddonSyntaxListEntry;

    #[test]
    fn type_creation() {
        // Typeでの独自の実装以外はConditionと同じ実装をしているため、省略
        let json = r#"{
        "id": 2167,
        "creator": 1,
        "title": "Timespan",
        "description": "A timespan is a difference of two different dates or times, e.g '10 minutes'. Timespans are always displayed as real life time, but can be defined as minecraft time, e.g. '5 minecraft days and 12 hours'.\nNOTE: Months always have the value of 30 days, and years of 365 days.\nSee date and time for the other time types of Skript.",
        "syntax_pattern": "time[ ]span[s]",
        "compatible_addon_version": "1.0, 2.6.1 (weeks, months, years)",
        "compatible_minecraft_version": "",
        "syntax_type": "type",
        "get_syntax_type_css_class": "syntax_title_box_type",
        "required_plugins": [],
        "addon": {
            "name": "Skript",
            "link_to_addon": "https://github.com/SkriptLang/Skript",
            "usage_score": 1480.1
        },
        "type_usage": "<number> [minecraft/mc/real/rl/irl] ticks/seconds/minutes/hours/days/weeks/months/years [[,/and] <more...>]\n[###:]##:##[.####] ([hours:]minutes:seconds[.milliseconds])",
        "return_type": "",
        "event_values": "",
        "json_id": "Timespan",
        "event_cancellable": false,
        "created_at": "2018-01-17T16:37:21.922274Z",
        "updated_at": "2022-01-26T08:25:24.566781Z",
        "entries": "",
        "keywords": null,
        "mark_as_removed": false,
        "removed_since": null
    }"#;
        let parsed: AbstractAddonSyntaxListEntry = serde_json::from_str(json).unwrap();
        let _type_ = Type::from_abstract_syntax_list_entry(&parsed).unwrap();
        // todo
    }
}
