use crate::{SnapshotError, raw};
use std::collections::BTreeMap;
use syntax_pattern_parser::syntax::PluralRules;
use syntaxes as model;

pub(crate) fn catalog(
    raw: raw::Snapshot,
    plural_rules: PluralRules,
) -> Result<model::Catalog, SnapshotError> {
    let raw::Snapshot {
        conditions,
        effects,
        events,
        expressions,
        sections,
        structures,
        types,
        functions,
        converters,
        comparators,
        event_values,
        properties,
        operators,
        operations,
        differences,
        classes,
        aliases,
        plural_rules: _,
    } = raw;

    let mut syntaxes = Vec::with_capacity(
        events.len()
            + conditions.len()
            + effects.len()
            + expressions.len()
            + types.len()
            + functions.len()
            + sections.len()
            + structures.len(),
    );

    for (index, value) in events.into_iter().enumerate() {
        syntaxes.push(model::Syntax::Event(model::Event {
            common: common("Events.json", index, value.common, &plural_rules)?,
            reference_events: class_names(value.reference_events),
            event_values: value.event_values.into_iter().map(event_value).collect(),
            cancellable: value.cancellable,
            has_on_prefix: value.has_on_prefix,
        }));
    }
    for (index, value) in conditions.into_iter().enumerate() {
        syntaxes.push(model::Syntax::Condition(model::Condition {
            common: common("Conditions.json", index, value, &plural_rules)?,
        }));
    }
    for (index, value) in effects.into_iter().enumerate() {
        syntaxes.push(model::Syntax::Effect(model::Effect {
            common: common("Effects.json", index, value, &plural_rules)?,
        }));
    }
    for (index, value) in expressions.into_iter().enumerate() {
        syntaxes.push(model::Syntax::Expression(model::Expression {
            common: common("Expressions.json", index, value.common, &plural_rules)?,
            return_type: value.return_type.map(Into::into),
            section_expression: value.section_expression,
            return_type_multiplicity: value.return_type_multiplicity.map(multiplicity),
            return_type_multiplicity_state: resolution_state(value.return_type_multiplicity_state),
            accepted_changers: value.accepted_changers.map(change_modes),
            accepted_changers_state: resolution_state(value.accepted_changers_state),
        }));
    }
    for value in types {
        syntaxes.push(model::Syntax::Type(type_data(value)));
    }
    for value in functions {
        syntaxes.push(model::Syntax::Function(function(value)?));
    }
    for (index, value) in sections.into_iter().enumerate() {
        syntaxes.push(model::Syntax::Section(model::Section {
            common: common("Sections.json", index, value.common, &plural_rules)?,
            loop_section: value.loop_section,
            effect_section: value.effect_section,
        }));
    }
    for (index, value) in structures.into_iter().enumerate() {
        syntaxes.push(model::Syntax::Structure(model::Structure {
            common: common("Structures.json", index, value.common, &plural_rules)?,
            entry_validator: value.entry_validator.map(entry_validator),
            node_type: value.node_type.map(node_type),
        }));
    }

    Ok(model::Catalog::new(model::CatalogParts {
        syntaxes,
        converters: converters.into_iter().map(converter).collect(),
        comparators: comparators.into_iter().map(comparator).collect(),
        event_values: event_values.into_iter().map(event_value).collect(),
        properties: properties.into_iter().map(property).collect(),
        operators: operators.into_iter().map(operator).collect(),
        operations: operations
            .into_iter()
            .map(|(sign, values)| (sign, values.into_iter().map(operation).collect::<Vec<_>>()))
            .collect(),
        differences: differences.into_iter().map(difference).collect(),
        classes: classes.into_iter().map(class).collect(),
        aliases: alias_registry(aliases),
        plural_rules,
    }))
}

fn common(
    file: &'static str,
    index: usize,
    value: raw::CommonSyntax,
    plural_rules: &PluralRules,
) -> Result<model::CommonSyntax, SnapshotError> {
    let patterns = value
        .patterns
        .into_iter()
        .enumerate()
        .map(|(pattern_index, source)| {
            let parsed =
                syntax_pattern_parser::syntax::parse(&source, plural_rules).map_err(|source| {
                    SnapshotError::Pattern {
                        path: format!("{file}[{index}].patterns[{pattern_index}]"),
                        source,
                    }
                })?;
            Ok(model::Pattern { source, parsed })
        })
        .collect::<Result<Vec<_>, SnapshotError>>()?;

    Ok(model::CommonSyntax {
        registration_order: value.registration_order,
        documentation: documentation(
            value.name,
            value.documentation_id,
            value.since,
            value.description,
            value.examples,
            value.keywords,
            value.requires,
        ),
        id: value.id,
        element_class: value.element_class.into(),
        super_class: value.super_class.map(Into::into),
        no_doc: value.no_doc,
        events: value.events.unwrap_or_default(),
        deprecated: value.deprecated,
        priority_name: value.priority_str,
        priority: value.priority.map(priority),
        patterns,
        addon: addon(value.addon),
        definition_id: value.definition_id.into(),
        registration_id: value.registration_id.into(),
        related_property: value.related_property,
        supported_events: value.supported_events.map(class_names),
        supported_events_state: value.supported_events_state.map(resolution_state),
        experimental_syntax: value.experimental_syntax.map(experimental_syntax),
        experimental_syntax_state: value.experimental_syntax_state.map(resolution_state),
        return_handler: value.return_handler.map(return_handler),
        return_handler_state: value.return_handler_state.map(resolution_state),
    })
}

fn documentation(
    name: Option<String>,
    documentation_id: Option<String>,
    since: Option<Vec<String>>,
    description: Option<Vec<String>>,
    examples: Option<Vec<String>>,
    keywords: Option<Vec<String>>,
    requires: Option<Vec<String>>,
) -> model::Documentation {
    model::Documentation {
        name,
        documentation_id,
        since: since.unwrap_or_default(),
        description: description.unwrap_or_default(),
        examples: examples.unwrap_or_default(),
        keywords: keywords.unwrap_or_default(),
        requires: requires.unwrap_or_default(),
    }
}

fn type_data(value: raw::Type) -> model::Type {
    model::Type {
        type_parse_order: value.type_parse_order,
        documentation: documentation(
            value.name,
            value.documentation_id,
            value.since,
            value.description,
            value.examples,
            value.keywords,
            value.requires,
        ),
        addon: addon(value.addon),
        definition_id: value.definition_id.into(),
        registration_id: value.registration_id.into(),
        has_docs: value.has_docs,
        changer: value.changer.map(change_modes),
        original_class: value.original_class.into(),
        class_type: class_kind(value.class_type),
        code_name: value.code_name.into(),
        super_class: value.super_class.map(Into::into),
        interfaces: class_names(value.interfaces),
        assignable_to: value.assignable_to.into_iter().map(Into::into).collect(),
        user_input_patterns: value.user_input_patterns.unwrap_or_default(),
        noun: model::Noun {
            key: value.noun.key,
            value: value.noun.value,
            singular: value.noun.singular,
            plural: value.noun.plural,
            gender: value.noun.gender,
            gender_id: value.noun.gender_id,
        },
        serialize_as: value.serialize_as.map(Into::into),
        usage: value.usage.unwrap_or_default(),
        default_expression_class: value.default_expression_class.map(Into::into),
        has_parser: value.has_parser,
        has_serializer: value.has_serializer,
        has_supplier: value.has_supplier,
        properties: value.properties,
        before: value
            .before
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect(),
        after: value
            .after
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect(),
    }
}

fn function(value: raw::Function) -> Result<model::Function, SnapshotError> {
    let parameters = value
        .parameters
        .into_iter()
        .enumerate()
        .map(|(parameter_index, parameter)| {
            let modifiers = parameter
                .modifiers
                .into_iter()
                .enumerate()
                .map(|(modifier_index, modifier)| {
                    let path = format!(
                        "Functions.json[{}].parameters[{parameter_index}].modifiers[{modifier_index}]",
                        value.registration_order
                    );
                    match modifier.kind {
                        raw::ParameterModifierKind::Optional => Ok(model::ParameterModifier::Optional),
                        raw::ParameterModifierKind::Keyed => Ok(model::ParameterModifier::Keyed),
                        raw::ParameterModifierKind::Unknown => Ok(model::ParameterModifier::Unknown),
                        raw::ParameterModifierKind::Range => Ok(model::ParameterModifier::Range {
                            min: modifier.min.ok_or_else(|| SnapshotError::validation(&path, "range modifier is missing min"))?,
                            max: modifier.max.ok_or_else(|| SnapshotError::validation(&path, "range modifier is missing max"))?,
                        }),
                    }
                })
                .collect::<Result<Vec<_>, SnapshotError>>()?;
            Ok(model::FunctionParameter {
                name: parameter.name,
                parameter_type: parameter.parameter_type.into(),
                modifiers,
                single: parameter.single,
            })
        })
        .collect::<Result<Vec<_>, SnapshotError>>()?;

    let name = value.name.ok_or_else(|| {
        SnapshotError::validation(
            format!("Functions.json[{}].name", value.registration_order),
            "function name is required",
        )
    })?;
    Ok(model::Function {
        registration_order: value.registration_order,
        name: name.clone(),
        documentation: documentation(
            Some(name),
            None,
            value.since,
            value.description,
            value.examples,
            value.keywords,
            value.requires,
        ),
        return_type: value.return_type.map(Into::into),
        return_type_is_single: value.return_type_is_single,
        parameters,
        addon: addon(value.addon),
        definition_id: value.definition_id.into(),
        registration_id: value.registration_id.into(),
    })
}

fn event_value(value: raw::EventValue) -> model::EventValue {
    model::EventValue {
        event_class: value.event_class.into(),
        value_class: value.value_class.into(),
        time: value.time,
        exclude_error_message: value.exclude_error_message,
        excludes: value.excludes.map(class_names),
        resolution_order: value.resolution_order,
        registration_order: value.registration_order,
        patterns: value.patterns,
        accepted_changers: value.accepted_changers.map(change_modes),
        context_dependent: value.context_dependent,
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn converter(value: raw::Converter) -> model::Converter {
    model::Converter {
        from: value.from.into(),
        to: value.to.into(),
        flags: value.flags,
        registration_order: value.registration_order,
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn comparator(value: raw::Comparator) -> model::Comparator {
    model::Comparator {
        registration_order: value.registration_order,
        first_type: value.first_type.into(),
        second_type: value.second_type.into(),
        supports_ordering: value.supports_ordering,
        supports_inversion: value.supports_inversion,
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn property(value: raw::Property) -> model::Property {
    model::Property {
        name: value.name,
        documentation_id: value.documentation_id,
        description: value.description,
        since: value.since.unwrap_or_default(),
        handler_class: value.handler_class.into(),
        related_types: value.related_types.into_iter().map(type_property).collect(),
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn type_property(value: raw::TypeProperty) -> model::TypeProperty {
    model::TypeProperty {
        type_code_name: value.type_code_name.into(),
        type_class: value.type_class.into(),
        description: value.description,
        provider: value.provider.map(addon),
        handler_class: value.handler_class.into(),
        handler_kind: match value.handler_kind {
            raw::PropertyHandlerKind::Expression => model::PropertyHandlerKind::Expression,
            raw::PropertyHandlerKind::Condition => model::PropertyHandlerKind::Condition,
            raw::PropertyHandlerKind::Contains => model::PropertyHandlerKind::Contains,
            raw::PropertyHandlerKind::TypedValue => model::PropertyHandlerKind::TypedValue,
            raw::PropertyHandlerKind::Wxyz => model::PropertyHandlerKind::Wxyz,
            raw::PropertyHandlerKind::Custom => model::PropertyHandlerKind::Custom,
        },
        return_type: value.return_type.map(Into::into),
        possible_return_types: value.possible_return_types.map(class_names),
        accepted_changers: value.accepted_changers.map(change_modes),
        requires_source_expression_change: value.requires_source_expression_change,
        expression_metadata_state: value.expression_metadata_state.map(resolution_state),
        element_types: value.element_types.map(class_names),
        supported_axes: value.supported_axes,
    }
}

fn operator(value: raw::Operator) -> model::Operator {
    model::Operator {
        sign: value.sign,
        priority: priority(value.priority),
        key: value.key,
        registration_order: value.registration_order,
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn operation(value: raw::Operation) -> model::Operation {
    model::Operation {
        operator_sign: value.operator_sign,
        left: value.left.into(),
        right: value.right.into(),
        return_type: value.return_type.into(),
        registration_order: value.registration_order,
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn difference(value: raw::Difference) -> model::Difference {
    model::Difference {
        input_type: value.input_type.into(),
        return_type: value.return_type.into(),
        registration_order: value.registration_order,
        addon: addon(value.addon),
        registration_id: value.registration_id.into(),
    }
}

fn class(value: raw::Class) -> model::Class {
    model::Class {
        name: value.name.into(),
        binary_name: value.binary_name,
        kind: class_kind(value.kind),
        super_class: value.super_class.map(Into::into),
        interfaces: class_names(value.interfaces),
        component_type: value.component_type.map(Into::into),
        provider: value.provider.map(addon),
    }
}

fn alias_registry(value: raw::Aliases) -> model::AliasRegistry {
    model::AliasRegistry {
        aliases: value.aliases,
        targets: value
            .targets
            .into_iter()
            .map(|target| model::AliasTarget {
                amount: target.amount,
                all: target.all,
                types: target
                    .types
                    .into_iter()
                    .map(|item| model::AliasItem {
                        material: item.material,
                        minecraft_id: item.minecraft_id,
                        durability: item.durability,
                        plain: item.plain,
                        alias: item.alias,
                        block_values: item.block_values,
                        item_meta: item.item_meta,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn entry_validator(value: raw::EntryValidator) -> model::EntryValidator {
    model::EntryValidator {
        entry_data: value
            .entry_data
            .into_iter()
            .map(|entry| model::EntryData {
                key: entry.key,
                default_value: entry.default_value,
                optional: entry.optional,
                multiple: entry.multiple,
                entry_data_class: entry.entry_data_class.into(),
                kind: match entry.kind {
                    raw::EntryKind::Literal => model::EntryKind::Literal,
                    raw::EntryKind::VariableString => model::EntryKind::VariableString,
                    raw::EntryKind::Expression => model::EntryKind::Expression,
                    raw::EntryKind::Trigger => model::EntryKind::Trigger,
                    raw::EntryKind::Container => model::EntryKind::Container,
                    raw::EntryKind::Section => model::EntryKind::Section,
                    raw::EntryKind::KeyValue => model::EntryKind::KeyValue,
                    raw::EntryKind::Unknown => model::EntryKind::Unknown,
                },
                separator: entry.separator,
                value_type: entry.value_type.map(Into::into),
                string_mode: entry.string_mode,
                return_types: class_names(entry.return_types.unwrap_or_default()),
                flags: entry.flags,
                nested_validator: entry.nested_validator.map(entry_validator),
            })
            .collect(),
    }
}

fn experimental_syntax(value: raw::ExperimentalSyntax) -> model::ExperimentalSyntax {
    let map = |value: raw::Experiment| model::Experiment {
        code_name: value.code_name,
        phase: value.phase,
        known: value.known,
    };
    model::ExperimentalSyntax {
        required: value.required.into_iter().map(map).collect(),
        disallowed: value.disallowed.into_iter().map(map).collect(),
        error_message: value.error_message,
    }
}

fn return_handler(value: raw::ReturnHandler) -> model::ReturnHandler {
    model::ReturnHandler {
        return_value_type: value.return_value_type.map(Into::into),
        single_return_value: value.single_return_value,
    }
}

fn addon(value: raw::Addon) -> model::Addon {
    model::Addon {
        name: value.name,
        version: value.version,
    }
}

fn priority(value: raw::Priority) -> model::Priority {
    model::Priority {
        after: value.after.into_iter().map(priority).collect(),
        before: value.before.into_iter().map(priority).collect(),
    }
}

fn change_modes(value: raw::ChangeModes) -> model::ChangeModes {
    value
        .into_iter()
        .map(|(mode, classes)| {
            let mode = match mode {
                raw::ChangeMode::Add => model::ChangeMode::Add,
                raw::ChangeMode::Set => model::ChangeMode::Set,
                raw::ChangeMode::Remove => model::ChangeMode::Remove,
                raw::ChangeMode::RemoveAll => model::ChangeMode::RemoveAll,
                raw::ChangeMode::Delete => model::ChangeMode::Delete,
                raw::ChangeMode::Reset => model::ChangeMode::Reset,
            };
            (mode, class_names(classes))
        })
        .collect::<BTreeMap<_, _>>()
}

fn class_names(values: Vec<String>) -> Vec<model::ClassName> {
    values.into_iter().map(Into::into).collect()
}

fn resolution_state(value: raw::ResolutionState) -> model::ResolutionState {
    match value {
        raw::ResolutionState::Resolved => model::ResolutionState::Resolved,
        raw::ResolutionState::Unresolved => model::ResolutionState::Unresolved,
    }
}

fn multiplicity(value: raw::Multiplicity) -> model::Multiplicity {
    match value {
        raw::Multiplicity::Single => model::Multiplicity::Single,
        raw::Multiplicity::Multiple => model::Multiplicity::Multiple,
        raw::Multiplicity::Both => model::Multiplicity::Both,
    }
}

fn class_kind(value: raw::ClassKind) -> model::ClassKind {
    match value {
        raw::ClassKind::Annotation => model::ClassKind::Annotation,
        raw::ClassKind::Enum => model::ClassKind::Enum,
        raw::ClassKind::Interface => model::ClassKind::Interface,
        raw::ClassKind::Array => model::ClassKind::Array,
        raw::ClassKind::Primitive => model::ClassKind::Primitive,
        raw::ClassKind::Record => model::ClassKind::Record,
        raw::ClassKind::Sealed => model::ClassKind::Sealed,
        raw::ClassKind::Synthetic => model::ClassKind::Synthetic,
        raw::ClassKind::MemberClass => model::ClassKind::MemberClass,
        raw::ClassKind::LocalClass => model::ClassKind::LocalClass,
        raw::ClassKind::AnonymousClass => model::ClassKind::AnonymousClass,
        raw::ClassKind::Class => model::ClassKind::Class,
    }
}

fn node_type(value: raw::NodeType) -> model::NodeType {
    match value {
        raw::NodeType::Simple => model::NodeType::Simple,
        raw::NodeType::Section => model::NodeType::Section,
        raw::NodeType::Both => model::NodeType::Both,
    }
}
