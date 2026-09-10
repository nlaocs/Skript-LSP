use skript_parser::{EventCandidate, ExpressionParseContext, PatternCapture};
use std::collections::{BTreeMap, BTreeSet};
use syntaxes::{Catalog, ChangeMode, DynamicSyntaxSnapshot, SyntaxKind};

/// Event context selected for later Effect, Condition, and Expression parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventContext {
    /// Normalized Event header supplied by the user.
    pub input: String,
    /// Stable SSG identity shared by equivalent registrations.
    pub definition_id: String,
    /// Stable SSG identity for the exact selected registration.
    pub registration_id: String,
    /// Pattern selected while parsing an Event header.
    pub pattern_index: usize,
    /// Original SSG Event pattern selected from the catalog.
    pub pattern: String,
    /// Java implementation class of the selected Event registration.
    pub element_class: Option<String>,
    /// Bukkit event classes visible to event-restricted syntax and event values.
    pub reference_events: Vec<String>,
    /// Bukkit event classes declared by the selected registration before addon updates.
    pub registered_reference_events: Vec<String>,
    /// Whether the Event is known to be cancellable.
    pub cancellable: Option<bool>,
    /// Whether the Event accepts an explicit priority modifier.
    pub priority_supported: Option<bool>,
    /// Listening behavior explicitly selected by the StructEvent header.
    pub listening_behavior: Option<String>,
    /// Bukkit Event priority explicitly selected by the StructEvent header.
    pub event_priority: Option<String>,
    /// Addon that registered the Event when it came from the SSG catalog.
    pub addon: Option<EventAddon>,
    /// Opaque handler selected for a dynamic Event registration.
    pub handler: Option<String>,
    /// Event registration metadata available to addon consumers.
    pub event_metadata: BTreeMap<String, String>,
    /// StructEvent metadata after ordered WASM hooks ran.
    pub structure_metadata: BTreeMap<String, String>,
    /// Event values available through the selected Bukkit event classes.
    pub event_values: Vec<EventValueContext>,
    /// Diagnostics emitted while the Event Structure hook established context.
    pub diagnostics: Vec<EventContextDiagnostic>,
    /// WASM component failures retained from Event context selection.
    pub component_failures: Vec<EventContextComponentFailure>,
    captures: Vec<PatternCapture>,
    pub(crate) parser_context: ExpressionParseContext,
}

impl EventContext {
    /// Returns the regex and typed captures retained from the selected Event pattern.
    pub fn captures(&self) -> &[PatternCapture] {
        &self.captures
    }

    pub(crate) fn from_candidate(
        catalog: &Catalog,
        input: String,
        event: EventCandidate,
        parser_context: ExpressionParseContext,
        structure_metadata: BTreeMap<String, String>,
        diagnostics: Vec<EventContextDiagnostic>,
        component_failures: Vec<EventContextComponentFailure>,
    ) -> Self {
        let addon = catalog
            .events()
            .find(|candidate| {
                candidate.common.registration_id.as_str() == event.matched.registration_id
            })
            .map(|event| EventAddon {
                name: event.common.addon.name.clone(),
                version: event.common.addon.version.clone(),
            });
        let registered_reference_events = event
            .reference_events
            .iter()
            .map(|class| class.as_str().to_owned())
            .collect::<Vec<_>>();
        let reference_events = parser_context
            .event_classes
            .iter()
            .map(|class| class.as_str().to_owned())
            .collect::<Vec<_>>();
        let listening_behavior =
            non_empty_context_value(&parser_context, "core.structure.event.listening-behavior");
        let event_priority =
            non_empty_context_value(&parser_context, "core.structure.event.priority");
        let cancellable = effective_boolean(&parser_context, "core.structure.event.cancellable")
            .unwrap_or(event.cancellable);
        let priority_supported =
            effective_boolean(&parser_context, "core.structure.event.priority-supported")
                .unwrap_or(event.priority_supported);
        let captures = event.matched.matched.captures.clone();
        Self {
            input,
            definition_id: event.matched.definition_id,
            registration_id: event.matched.registration_id,
            pattern_index: event.matched.pattern_index,
            pattern: event.matched.pattern,
            element_class: event.element_class.map(|class| class.0),
            event_values: event_values_for(catalog, &reference_events),
            reference_events,
            registered_reference_events,
            cancellable,
            priority_supported,
            listening_behavior,
            event_priority,
            addon,
            handler: event.handler,
            event_metadata: event.metadata,
            structure_metadata,
            diagnostics,
            component_failures,
            captures,
            parser_context,
        }
    }
}

/// Addon identity displayed for a selected Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAddon {
    pub name: String,
    pub version: String,
}

/// Event value made available by the selected Event context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventValueContext {
    pub event_class: String,
    pub value_class: String,
    pub time: i32,
    pub exclude_error_message: Option<String>,
    pub excludes: Option<Vec<String>>,
    pub resolution_order: usize,
    pub registration_order: Option<usize>,
    pub registration_id: String,
    pub patterns: Option<Vec<String>>,
    pub accepted_changers: Option<Vec<EventValueChangerContext>>,
    pub context_dependent: Option<bool>,
    pub has_custom_input_validator: Option<bool>,
    pub has_custom_event_validator: Option<bool>,
    pub addon: EventAddon,
}

/// Changer mode and Java value classes accepted by one Event value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventValueChangerContext {
    pub mode: String,
    pub accepted_classes: Vec<String>,
}

/// Diagnostic emitted while selecting an Event context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventContextDiagnostic {
    pub code: String,
    pub message: String,
    pub severity: String,
}

/// WASM component failure emitted while selecting an Event context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventContextComponentFailure {
    pub component_id: String,
    pub subscription_id: String,
    pub message: String,
}

/// Catalog Event displayed by `:events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSummary {
    pub definition_id: String,
    pub registration_id: String,
    pub element_class: Option<String>,
    pub addon: Option<EventAddon>,
    pub handler: Option<String>,
    pub patterns: Vec<String>,
}

pub(crate) fn event_summaries(
    catalog: &Catalog,
    dynamic: Option<&DynamicSyntaxSnapshot>,
) -> Vec<EventSummary> {
    let mut summaries = catalog
        .events()
        .map(|event| EventSummary {
            definition_id: event.common.definition_id.as_str().to_owned(),
            registration_id: event.common.registration_id.as_str().to_owned(),
            element_class: Some(event.common.element_class.as_str().to_owned()),
            addon: Some(EventAddon {
                name: event.common.addon.name.clone(),
                version: event.common.addon.version.clone(),
            }),
            handler: None,
            patterns: event
                .common
                .patterns
                .iter()
                .map(|pattern| pattern.source.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    if let Some(dynamic) = dynamic {
        summaries.extend(
            dynamic
                .definitions
                .values()
                .filter(|definition| definition.kind == SyntaxKind::Event)
                .map(|definition| {
                    let identity = definition.id.qualified();
                    EventSummary {
                        definition_id: identity.clone(),
                        registration_id: identity,
                        element_class: None,
                        addon: None,
                        handler: Some(definition.handler.clone()),
                        patterns: definition
                            .patterns
                            .iter()
                            .map(|pattern| pattern.source.clone())
                            .collect(),
                    }
                }),
        );
    }
    summaries
}

pub(crate) fn normalize_event_header(input: &str) -> Result<String, &'static str> {
    let mut input = input.trim();
    if input.len() >= 2 && input.starts_with('"') && input.ends_with('"') {
        input = &input[1..input.len() - 1];
    }
    input = input.trim();
    if let Some(without_colon) = input.strip_suffix(':') {
        input = without_colon.trim_end();
    }
    if input.is_empty() {
        return Err("Event header is empty");
    }
    if input.contains(['\r', '\n']) {
        return Err("Event header must be one line");
    }
    Ok(input.to_owned())
}

fn non_empty_context_value(context: &ExpressionParseContext, key: &str) -> Option<String> {
    context
        .values
        .get(key)
        .filter(|value| !value.is_empty())
        .cloned()
}

fn effective_boolean(context: &ExpressionParseContext, key: &str) -> Option<Option<bool>> {
    context.values.get(key).map(|value| match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

fn event_values_for(catalog: &Catalog, classes: &[String]) -> Vec<EventValueContext> {
    let mut seen = BTreeSet::new();
    classes
        .iter()
        .flat_map(|class| catalog.event_values_for(class))
        .filter(|value| seen.insert(value.registration_id.as_str().to_owned()))
        .map(|value| EventValueContext {
            event_class: value.event_class.as_str().to_owned(),
            value_class: value.value_class.as_str().to_owned(),
            time: value.time,
            exclude_error_message: value.exclude_error_message.clone(),
            excludes: value.excludes.as_ref().map(|classes| {
                classes
                    .iter()
                    .map(|class| class.as_str().to_owned())
                    .collect()
            }),
            resolution_order: value.resolution_order,
            registration_order: value.registration_order,
            registration_id: value.registration_id.as_str().to_owned(),
            patterns: value.patterns.clone(),
            accepted_changers: value.accepted_changers.as_ref().map(|changers| {
                changers
                    .iter()
                    .map(|(mode, classes)| EventValueChangerContext {
                        mode: change_mode_name(*mode).to_owned(),
                        accepted_classes: classes
                            .iter()
                            .map(|class| class.as_str().to_owned())
                            .collect(),
                    })
                    .collect()
            }),
            context_dependent: value.context_dependent,
            has_custom_input_validator: value.has_custom_input_validator,
            has_custom_event_validator: value.has_custom_event_validator,
            addon: EventAddon {
                name: value.addon.name.clone(),
                version: value.addon.version.clone(),
            },
        })
        .collect()
}

fn change_mode_name(mode: ChangeMode) -> &'static str {
    match mode {
        ChangeMode::Add => "ADD",
        ChangeMode::Set => "SET",
        ChangeMode::Remove => "REMOVE",
        ChangeMode::RemoveAll => "REMOVE_ALL",
        ChangeMode::Delete => "DELETE",
        ChangeMode::Reset => "RESET",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_optional_event_colons_and_repl_quotes() {
        assert_eq!(normalize_event_header("on join").unwrap(), "on join");
        assert_eq!(normalize_event_header("on join:").unwrap(), "on join");
        assert_eq!(normalize_event_header("\"join:\"").unwrap(), "join");
        assert!(normalize_event_header("  :  ").is_err());
        assert!(normalize_event_header("on join\nsend 1").is_err());
    }

    #[test]
    fn effective_boolean_distinguishes_missing_and_unresolved_values() {
        let mut context = ExpressionParseContext::default();
        assert_eq!(effective_boolean(&context, "event.flag"), None);
        context
            .values
            .insert("event.flag".to_owned(), "true".to_owned());
        assert_eq!(effective_boolean(&context, "event.flag"), Some(Some(true)));
        context
            .values
            .insert("event.flag".to_owned(), "unresolved".to_owned());
        assert_eq!(effective_boolean(&context, "event.flag"), Some(None));
    }
}
