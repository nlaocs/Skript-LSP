//! Semantic validation for individual SSG files and cross-file references.
//!
//! Validation is deliberately separate from deserialization so malformed structure
//! and inconsistent-but-valid JSON produce precise, stable errors.

use crate::SnapshotError;
use crate::raw::{self, EventValueApi, ResolutionState, SyntaxKind};
use std::collections::{HashMap, HashSet};

pub(crate) fn manifest(
    manifest: &raw::Manifest,
    expected_files: &[&str],
) -> Result<(), SnapshotError> {
    let actual = manifest
        .files
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if actual != expected_files {
        return Err(SnapshotError::ManifestFiles {
            message: format!("expected {expected_files:?}, found {actual:?}"),
        });
    }

    validate_sha("Manifest.json.snapshotId", &manifest.snapshot_id)?;
    validate_sha("Manifest.json.contentDigest", &manifest.content_digest)?;
    contiguous(
        "Manifest.json.plugins",
        manifest.plugins.iter().map(|plugin| plugin.load_order),
    )?;
    for (index, plugin) in manifest.plugins.iter().enumerate() {
        non_blank(
            &format!("Manifest.json.plugins[{index}].name"),
            &plugin.name,
        )?;
        non_blank(
            &format!("Manifest.json.plugins[{index}].version"),
            &plugin.version,
        )?;
        non_blank(
            &format!("Manifest.json.plugins[{index}].main"),
            &plugin.main,
        )?;
        if let Some(hash) = &plugin.jar_sha256 {
            validate_sha(&format!("Manifest.json.plugins[{index}].jarSha256"), hash)?;
        }
    }
    if manifest.capabilities.aliases.collected && !manifest.capabilities.aliases.supported {
        return Err(SnapshotError::validation(
            "Manifest.json.capabilities.aliases",
            "collected cannot be true when supported is false",
        ));
    }
    Ok(())
}

pub(crate) fn snapshot(
    manifest: &raw::Manifest,
    snapshot: &raw::Snapshot,
) -> Result<(), SnapshotError> {
    validate_capability_roots(manifest, snapshot)?;
    validate_orders(snapshot)?;
    validate_syntaxes(snapshot)?;

    let class_paths = unique_classes(snapshot)?;
    let type_paths = unique_types(snapshot)?;
    let property_names = snapshot
        .properties
        .iter()
        .map(|property| property.name.as_str())
        .collect::<HashSet<_>>();
    let event_value_ids = snapshot
        .event_values
        .iter()
        .map(|value| value.registration_id.as_str())
        .collect::<HashSet<_>>();
    let operator_signs = snapshot
        .operators
        .iter()
        .map(|operator| operator.sign.as_str())
        .collect::<HashSet<_>>();

    validate_class_graph(snapshot, &class_paths)?;
    validate_type_references(snapshot, &class_paths, &type_paths, &property_names)?;
    validate_syntax_references(snapshot, &class_paths, &event_value_ids)?;
    validate_function_references(snapshot, &class_paths)?;
    validate_registry_references(snapshot, &class_paths, &type_paths, &operator_signs)?;
    validate_aliases(snapshot)?;
    validate_event_value_shape(manifest, snapshot)?;
    Ok(())
}

fn validate_capability_roots(
    manifest: &raw::Manifest,
    snapshot: &raw::Snapshot,
) -> Result<(), SnapshotError> {
    let kinds = &manifest.capabilities.syntax_kinds;
    require_empty_when_unsupported("Conditions.json", kinds.conditions, &snapshot.conditions)?;
    require_empty_when_unsupported("Effects.json", kinds.effects, &snapshot.effects)?;
    require_empty_when_unsupported("Events.json", kinds.events, &snapshot.events)?;
    require_empty_when_unsupported("Expressions.json", kinds.expressions, &snapshot.expressions)?;
    require_empty_when_unsupported("Types.json", kinds.types, &snapshot.types)?;
    require_empty_when_unsupported("Functions.json", kinds.functions, &snapshot.functions)?;
    require_empty_when_unsupported("Sections.json", kinds.sections, &snapshot.sections)?;
    require_empty_when_unsupported("Structures.json", kinds.structures, &snapshot.structures)?;
    require_empty_when_unsupported("Properties.json", kinds.properties, &snapshot.properties)?;
    require_empty_when_unsupported("Converters.json", kinds.converters, &snapshot.converters)?;
    require_empty_when_unsupported("Comparators.json", kinds.comparators, &snapshot.comparators)?;
    require_empty_when_unsupported(
        "EventValues.json",
        kinds.event_values,
        &snapshot.event_values,
    )?;
    if !kinds.arithmetic
        && (!snapshot.operators.is_empty()
            || !snapshot.operations.is_empty()
            || !snapshot.differences.is_empty())
    {
        return Err(SnapshotError::validation(
            "Manifest.json.capabilities.syntaxKinds.arithmetic",
            "unsupported arithmetic capability requires empty arithmetic roots",
        ));
    }
    if !manifest.capabilities.aliases.collected
        && (!snapshot.aliases.aliases.is_empty() || !snapshot.aliases.targets.is_empty())
    {
        return Err(SnapshotError::validation(
            "Aliases.json",
            "aliases must be empty when aliases.collected is false",
        ));
    }
    Ok(())
}

fn validate_orders(snapshot: &raw::Snapshot) -> Result<(), SnapshotError> {
    contiguous(
        "Conditions.json",
        snapshot
            .conditions
            .iter()
            .map(|value| value.registration_order),
    )?;
    contiguous(
        "Effects.json",
        snapshot
            .effects
            .iter()
            .map(|value| value.registration_order),
    )?;
    contiguous(
        "Events.json",
        snapshot
            .events
            .iter()
            .map(|value| value.common.registration_order),
    )?;
    contiguous(
        "Expressions.json",
        snapshot
            .expressions
            .iter()
            .map(|value| value.common.registration_order),
    )?;
    contiguous(
        "Sections.json",
        snapshot
            .sections
            .iter()
            .map(|value| value.common.registration_order),
    )?;
    contiguous(
        "Structures.json",
        snapshot
            .structures
            .iter()
            .map(|value| value.common.registration_order),
    )?;
    contiguous(
        "Types.json",
        snapshot.types.iter().map(|value| value.type_parse_order),
    )?;
    contiguous(
        "Functions.json",
        snapshot
            .functions
            .iter()
            .map(|value| value.registration_order),
    )?;
    contiguous(
        "Converters.json",
        snapshot
            .converters
            .iter()
            .map(|value| value.registration_order),
    )?;
    contiguous(
        "Comparators.json",
        snapshot
            .comparators
            .iter()
            .map(|value| value.registration_order),
    )?;
    contiguous(
        "EventValues.json.resolutionOrder",
        snapshot
            .event_values
            .iter()
            .map(|value| value.resolution_order),
    )?;
    contiguous(
        "Operators.json",
        snapshot
            .operators
            .iter()
            .map(|value| value.registration_order),
    )?;
    contiguous(
        "Differences.json",
        snapshot
            .differences
            .iter()
            .map(|value| value.registration_order),
    )?;
    Ok(())
}

fn validate_syntaxes(snapshot: &raw::Snapshot) -> Result<(), SnapshotError> {
    let mut registration_ids = HashMap::new();
    for (file, expected, values) in [
        (
            "Conditions.json",
            SyntaxKind::Condition,
            &snapshot.conditions,
        ),
        ("Effects.json", SyntaxKind::Effect, &snapshot.effects),
    ] {
        for (index, value) in values.iter().enumerate() {
            validate_common(file, index, value, expected, &mut registration_ids)?;
        }
    }
    for (index, value) in snapshot.events.iter().enumerate() {
        validate_common(
            "Events.json",
            index,
            &value.common,
            SyntaxKind::Event,
            &mut registration_ids,
        )?;
    }
    for (index, value) in snapshot.expressions.iter().enumerate() {
        validate_common(
            "Expressions.json",
            index,
            &value.common,
            SyntaxKind::Expression,
            &mut registration_ids,
        )?;
        state_pair(
            &format!("Expressions.json[{index}].returnTypeMultiplicity"),
            value.return_type_multiplicity.is_some(),
            Some(value.return_type_multiplicity_state),
        )?;
        state_pair(
            &format!("Expressions.json[{index}].acceptedChangers"),
            value.accepted_changers.is_some(),
            Some(value.accepted_changers_state),
        )?;
    }
    for (index, value) in snapshot.sections.iter().enumerate() {
        validate_common(
            "Sections.json",
            index,
            &value.common,
            SyntaxKind::Section,
            &mut registration_ids,
        )?;
    }
    for (index, value) in snapshot.structures.iter().enumerate() {
        validate_common(
            "Structures.json",
            index,
            &value.common,
            SyntaxKind::Structure,
            &mut registration_ids,
        )?;
    }
    for (index, value) in snapshot.types.iter().enumerate() {
        unique_id(
            &mut registration_ids,
            &format!("Types.json[{index}].registrationId"),
            &value.registration_id,
        )?;
    }
    let mut function_signatures = HashMap::new();
    for (index, value) in snapshot.functions.iter().enumerate() {
        unique_id(
            &mut registration_ids,
            &format!("Functions.json[{index}].registrationId"),
            &value.registration_id,
        )?;
        non_blank(
            &format!("Functions.json[{index}].definitionId"),
            &value.definition_id,
        )?;
        let name_path = format!("Functions.json[{index}].name");
        let name = value
            .name
            .as_deref()
            .ok_or_else(|| SnapshotError::validation(&name_path, "function name is required"))?;
        non_blank(&name_path, name)?;

        for (parameter_index, parameter) in value.parameters.iter().enumerate() {
            non_blank(
                &format!("Functions.json[{index}].parameters[{parameter_index}].name"),
                &parameter.name,
            )?;
            for (modifier_index, modifier) in parameter.modifiers.iter().enumerate() {
                let range = matches!(modifier.kind, raw::ParameterModifierKind::Range);
                if range != (modifier.min.is_some() && modifier.max.is_some()) {
                    return Err(SnapshotError::validation(
                        format!(
                            "Functions.json[{index}].parameters[{parameter_index}].modifiers[{modifier_index}]"
                        ),
                        "range modifiers require min and max; other modifiers must omit them",
                    ));
                }
            }
        }

        let signature = (
            name.to_owned(),
            value
                .parameters
                .iter()
                .map(|parameter| parameter.parameter_type.clone())
                .collect::<Vec<_>>(),
        );
        if let Some(previous) = function_signatures.insert(signature, name_path.clone()) {
            return Err(SnapshotError::validation(
                name_path,
                format!("duplicate function signature; first declared at {previous}"),
            ));
        }
    }
    Ok(())
}

fn validate_common<'a>(
    file: &str,
    index: usize,
    value: &'a raw::CommonSyntax,
    expected: SyntaxKind,
    registration_ids: &mut HashMap<&'a str, String>,
) -> Result<(), SnapshotError> {
    let path = format!("{file}[{index}]");
    if value.kind != expected {
        return Err(SnapshotError::validation(
            format!("{path}.kind"),
            format!("expected {expected:?}, found {:?}", value.kind),
        ));
    }
    non_blank(&format!("{path}.elementClass"), &value.element_class)?;
    non_blank(&format!("{path}.definitionId"), &value.definition_id)?;
    unique_id(
        registration_ids,
        &format!("{path}.registrationId"),
        &value.registration_id,
    )?;
    state_pair(
        &format!("{path}.supportedEvents"),
        value.supported_events.is_some(),
        value.supported_events_state,
    )?;
    state_pair(
        &format!("{path}.experimentalSyntax"),
        value.experimental_syntax.is_some(),
        value.experimental_syntax_state,
    )?;
    state_pair(
        &format!("{path}.returnHandler"),
        value.return_handler.is_some(),
        value.return_handler_state,
    )?;
    Ok(())
}

fn unique_classes(snapshot: &raw::Snapshot) -> Result<HashMap<&str, String>, SnapshotError> {
    let mut result = HashMap::new();
    for (index, class) in snapshot.classes.iter().enumerate() {
        let path = format!("ClassHierarchy.json[{index}].name");
        if let Some(previous) = result.insert(class.name.as_str(), path.clone()) {
            return Err(SnapshotError::validation(
                path,
                format!("duplicate class name; first declared at {previous}"),
            ));
        }
    }
    Ok(result)
}

fn unique_types(snapshot: &raw::Snapshot) -> Result<HashMap<&str, String>, SnapshotError> {
    let mut result = HashMap::new();
    for (index, value) in snapshot.types.iter().enumerate() {
        let path = format!("Types.json[{index}].codeName");
        non_blank(&path, &value.code_name)?;
        if let Some(previous) = result.insert(value.code_name.as_str(), path.clone()) {
            return Err(SnapshotError::validation(
                path,
                format!("duplicate type code name; first declared at {previous}"),
            ));
        }
    }
    Ok(result)
}

fn validate_class_graph(
    snapshot: &raw::Snapshot,
    classes: &HashMap<&str, String>,
) -> Result<(), SnapshotError> {
    for (index, class) in snapshot.classes.iter().enumerate() {
        if let Some(parent) = &class.super_class {
            class_ref(
                classes,
                &format!("ClassHierarchy.json[{index}].superClass"),
                parent,
            )?;
        }
        for (interface_index, interface) in class.interfaces.iter().enumerate() {
            class_ref(
                classes,
                &format!("ClassHierarchy.json[{index}].interfaces[{interface_index}]"),
                interface,
            )?;
        }
        if let Some(component) = &class.component_type {
            class_ref(
                classes,
                &format!("ClassHierarchy.json[{index}].componentType"),
                component,
            )?;
        }
    }
    Ok(())
}

fn validate_type_references(
    snapshot: &raw::Snapshot,
    classes: &HashMap<&str, String>,
    types: &HashMap<&str, String>,
    properties: &HashSet<&str>,
) -> Result<(), SnapshotError> {
    for (index, value) in snapshot.types.iter().enumerate() {
        class_ref(
            classes,
            &format!("Types.json[{index}].originalClass"),
            &value.original_class,
        )?;
        for (field, referenced) in [
            ("superClass", value.super_class.as_ref()),
            ("serializeAs", value.serialize_as.as_ref()),
            (
                "defaultExpressionClass",
                value.default_expression_class.as_ref(),
            ),
        ] {
            if let Some(referenced) = referenced {
                class_ref(classes, &format!("Types.json[{index}].{field}"), referenced)?;
            }
        }
        for (reference_index, reference) in value.interfaces.iter().enumerate() {
            class_ref(
                classes,
                &format!("Types.json[{index}].interfaces[{reference_index}]"),
                reference,
            )?;
        }
        validate_change_modes(
            classes,
            &format!("Types.json[{index}].changer"),
            value.changer.as_ref(),
        )?;
        for (field, references) in [
            ("assignableTo", value.assignable_to.as_slice()),
            ("before", value.before.as_deref().unwrap_or_default()),
            ("after", value.after.as_deref().unwrap_or_default()),
        ] {
            for (reference_index, reference) in references.iter().enumerate() {
                type_ref(
                    types,
                    &format!("Types.json[{index}].{field}[{reference_index}]"),
                    reference,
                )?;
            }
        }
        for (property_index, property) in value.properties.iter().enumerate() {
            if !properties.contains(property.as_str()) {
                return Err(SnapshotError::validation(
                    format!("Types.json[{index}].properties[{property_index}]"),
                    format!("unknown property {property:?}"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_syntax_references(
    snapshot: &raw::Snapshot,
    classes: &HashMap<&str, String>,
    event_value_ids: &HashSet<&str>,
) -> Result<(), SnapshotError> {
    for (file, values) in [
        ("Conditions.json", &snapshot.conditions),
        ("Effects.json", &snapshot.effects),
    ] {
        for (index, value) in values.iter().enumerate() {
            validate_common_class_refs(classes, file, index, value)?;
        }
    }
    for (index, event) in snapshot.events.iter().enumerate() {
        validate_common_class_refs(classes, "Events.json", index, &event.common)?;
        for (reference_index, reference) in event.reference_events.iter().enumerate() {
            class_ref(
                classes,
                &format!("Events.json[{index}].referenceEvents[{reference_index}]"),
                reference,
            )?;
        }
        for (value_index, value) in event.event_values.iter().enumerate() {
            validate_event_value(
                classes,
                &format!("Events.json[{index}].eventValues[{value_index}]"),
                value,
            )?;
            if !event_value_ids.contains(value.registration_id.as_str()) {
                return Err(SnapshotError::validation(
                    format!("Events.json[{index}].eventValues[{value_index}].registrationId"),
                    "inline event value is absent from EventValues.json",
                ));
            }
        }
    }
    for (index, expression) in snapshot.expressions.iter().enumerate() {
        validate_common_class_refs(classes, "Expressions.json", index, &expression.common)?;
        if let Some(return_type) = &expression.return_type {
            class_ref(
                classes,
                &format!("Expressions.json[{index}].returnType"),
                return_type,
            )?;
        }
        validate_change_modes(
            classes,
            &format!("Expressions.json[{index}].acceptedChangers"),
            expression.accepted_changers.as_ref(),
        )?;
    }
    for (index, section) in snapshot.sections.iter().enumerate() {
        validate_common_class_refs(classes, "Sections.json", index, &section.common)?;
    }
    for (index, structure) in snapshot.structures.iter().enumerate() {
        validate_common_class_refs(classes, "Structures.json", index, &structure.common)?;
        if let Some(validator) = &structure.entry_validator {
            validate_entry_validator(
                classes,
                &format!("Structures.json[{index}].entryValidator"),
                validator,
            )?;
        }
    }
    Ok(())
}

fn validate_common_class_refs(
    classes: &HashMap<&str, String>,
    file: &str,
    index: usize,
    value: &raw::CommonSyntax,
) -> Result<(), SnapshotError> {
    class_ref(
        classes,
        &format!("{file}[{index}].elementClass"),
        &value.element_class,
    )?;
    if let Some(parent) = &value.super_class {
        class_ref(classes, &format!("{file}[{index}].superClass"), parent)?;
    }
    for (event_index, event) in value.supported_events.iter().flatten().enumerate() {
        class_ref(
            classes,
            &format!("{file}[{index}].supportedEvents[{event_index}]"),
            event,
        )?;
    }
    if let Some(return_type) = value
        .return_handler
        .as_ref()
        .and_then(|handler| handler.return_value_type.as_ref())
    {
        class_ref(
            classes,
            &format!("{file}[{index}].returnHandler.returnValueType"),
            return_type,
        )?;
    }
    Ok(())
}

fn validate_entry_validator(
    classes: &HashMap<&str, String>,
    path: &str,
    validator: &raw::EntryValidator,
) -> Result<(), SnapshotError> {
    for (index, entry) in validator.entry_data.iter().enumerate() {
        let entry_path = format!("{path}.entryData[{index}]");
        class_ref(
            classes,
            &format!("{entry_path}.entryDataClass"),
            &entry.entry_data_class,
        )?;
        if let Some(value_type) = &entry.value_type {
            class_ref(classes, &format!("{entry_path}.valueType"), value_type)?;
        }
        for (type_index, return_type) in entry.return_types.iter().flatten().enumerate() {
            class_ref(
                classes,
                &format!("{entry_path}.returnTypes[{type_index}]"),
                return_type,
            )?;
        }
        if let Some(nested) = &entry.nested_validator {
            validate_entry_validator(classes, &format!("{entry_path}.nestedValidator"), nested)?;
        }
    }
    Ok(())
}

fn validate_function_references(
    snapshot: &raw::Snapshot,
    classes: &HashMap<&str, String>,
) -> Result<(), SnapshotError> {
    for (index, function) in snapshot.functions.iter().enumerate() {
        if let Some(return_type) = &function.return_type {
            class_ref(
                classes,
                &format!("Functions.json[{index}].returnType"),
                return_type,
            )?;
        }
        for (parameter_index, parameter) in function.parameters.iter().enumerate() {
            class_ref(
                classes,
                &format!("Functions.json[{index}].parameters[{parameter_index}].type"),
                &parameter.parameter_type,
            )?;
        }
    }
    Ok(())
}

fn validate_registry_references(
    snapshot: &raw::Snapshot,
    classes: &HashMap<&str, String>,
    types: &HashMap<&str, String>,
    operator_signs: &HashSet<&str>,
) -> Result<(), SnapshotError> {
    for (index, value) in snapshot.event_values.iter().enumerate() {
        validate_event_value(classes, &format!("EventValues.json[{index}]"), value)?;
    }
    for (index, value) in snapshot.converters.iter().enumerate() {
        class_ref(
            classes,
            &format!("Converters.json[{index}].from"),
            &value.from,
        )?;
        class_ref(classes, &format!("Converters.json[{index}].to"), &value.to)?;
    }
    for (index, value) in snapshot.comparators.iter().enumerate() {
        class_ref(
            classes,
            &format!("Comparators.json[{index}].firstType"),
            &value.first_type,
        )?;
        class_ref(
            classes,
            &format!("Comparators.json[{index}].secondType"),
            &value.second_type,
        )?;
    }
    for (index, property) in snapshot.properties.iter().enumerate() {
        class_ref(
            classes,
            &format!("Properties.json[{index}].handlerClass"),
            &property.handler_class,
        )?;
        for (related_index, related) in property.related_types.iter().enumerate() {
            let path = format!("Properties.json[{index}].relatedTypes[{related_index}]");
            type_ref(
                types,
                &format!("{path}.typeCodeName"),
                &related.type_code_name,
            )?;
            class_ref(classes, &format!("{path}.typeClass"), &related.type_class)?;
            class_ref(
                classes,
                &format!("{path}.handlerClass"),
                &related.handler_class,
            )?;
            if let Some(value) = &related.return_type {
                class_ref(classes, &format!("{path}.returnType"), value)?;
            }
            for (class_index, value) in related
                .possible_return_types
                .iter()
                .flatten()
                .chain(related.element_types.iter().flatten())
                .enumerate()
            {
                class_ref(
                    classes,
                    &format!("{path}.classReferences[{class_index}]"),
                    value,
                )?;
            }
            validate_change_modes(
                classes,
                &format!("{path}.acceptedChangers"),
                related.accepted_changers.as_ref(),
            )?;
        }
    }
    for (sign, operations) in &snapshot.operations {
        if !operator_signs.contains(sign.as_str()) {
            return Err(SnapshotError::validation(
                format!("Operations.json.{sign}"),
                "operator sign is absent from Operators.json",
            ));
        }
        for (index, value) in operations.iter().enumerate() {
            let path = format!("Operations.json.{sign}[{index}]");
            if value.operator_sign != *sign {
                return Err(SnapshotError::validation(
                    format!("{path}.operatorSign"),
                    format!("expected {sign:?}"),
                ));
            }
            for (field, class) in [
                ("left", &value.left),
                ("right", &value.right),
                ("returnType", &value.return_type),
            ] {
                class_ref(classes, &format!("{path}.{field}"), class)?;
            }
        }
    }
    for (index, value) in snapshot.differences.iter().enumerate() {
        class_ref(
            classes,
            &format!("Differences.json[{index}].type"),
            &value.input_type,
        )?;
        class_ref(
            classes,
            &format!("Differences.json[{index}].returnType"),
            &value.return_type,
        )?;
    }
    Ok(())
}

fn validate_event_value(
    classes: &HashMap<&str, String>,
    path: &str,
    value: &raw::EventValue,
) -> Result<(), SnapshotError> {
    class_ref(classes, &format!("{path}.eventClass"), &value.event_class)?;
    class_ref(classes, &format!("{path}.valueClass"), &value.value_class)?;
    if !(-1..=1).contains(&value.time) {
        return Err(SnapshotError::validation(
            format!("{path}.time"),
            "expected -1, 0, or 1",
        ));
    }
    for (index, excluded) in value.excludes.iter().flatten().enumerate() {
        class_ref(classes, &format!("{path}.excludes[{index}]"), excluded)?;
    }
    validate_change_modes(
        classes,
        &format!("{path}.acceptedChangers"),
        value.accepted_changers.as_ref(),
    )
}

fn validate_event_value_shape(
    manifest: &raw::Manifest,
    snapshot: &raw::Snapshot,
) -> Result<(), SnapshotError> {
    for (index, value) in snapshot.event_values.iter().enumerate() {
        let path = format!("EventValues.json[{index}]");
        match manifest.capabilities.event_value_api {
            EventValueApi::Legacy
                if value.patterns.is_some()
                    || value.accepted_changers.is_some()
                    || value.context_dependent.is_some() =>
            {
                return Err(SnapshotError::validation(
                    path,
                    "legacy event values must omit modern fields",
                ));
            }
            EventValueApi::Modern215
                if value.patterns.is_none()
                    || value.accepted_changers.is_none()
                    || value.context_dependent.is_some() =>
            {
                return Err(SnapshotError::validation(
                    path,
                    "modern-2.15 requires patterns and acceptedChangers but omits contextDependent",
                ));
            }
            EventValueApi::Modern216
                if value.patterns.is_none()
                    || value.accepted_changers.is_none()
                    || value.context_dependent.is_none() =>
            {
                return Err(SnapshotError::validation(
                    path,
                    "modern-2.16 requires patterns, acceptedChangers, and contextDependent",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_aliases(snapshot: &raw::Snapshot) -> Result<(), SnapshotError> {
    let mut referenced = vec![false; snapshot.aliases.targets.len()];
    for (text, index) in &snapshot.aliases.aliases {
        let Some(slot) = referenced.get_mut(*index) else {
            return Err(SnapshotError::validation(
                format!("Aliases.json.aliases.{text}"),
                format!("target index {index} is out of range"),
            ));
        };
        *slot = true;
    }
    if let Some(index) = referenced.iter().position(|value| !value) {
        return Err(SnapshotError::validation(
            format!("Aliases.json.targets[{index}]"),
            "target is not referenced by any alias",
        ));
    }
    Ok(())
}

fn validate_change_modes(
    classes: &HashMap<&str, String>,
    path: &str,
    modes: Option<&raw::ChangeModes>,
) -> Result<(), SnapshotError> {
    for (mode, values) in modes.into_iter().flatten() {
        for (index, value) in values.iter().enumerate() {
            class_ref(classes, &format!("{path}.{mode:?}[{index}]"), value)?;
        }
    }
    Ok(())
}

fn state_pair(
    path: &str,
    has_value: bool,
    state: Option<ResolutionState>,
) -> Result<(), SnapshotError> {
    let valid = matches!(
        (has_value, state),
        (false, None)
            | (true, Some(ResolutionState::Resolved))
            | (false, Some(ResolutionState::Unresolved))
    );
    if valid {
        Ok(())
    } else {
        Err(SnapshotError::validation(
            path,
            format!("value/state mismatch: value={has_value}, state={state:?}"),
        ))
    }
}

fn class_ref(
    classes: &HashMap<&str, String>,
    path: &str,
    value: &str,
) -> Result<(), SnapshotError> {
    if classes.contains_key(value) {
        Ok(())
    } else {
        Err(SnapshotError::validation(
            path,
            format!("unknown class {value:?}"),
        ))
    }
}

fn type_ref(types: &HashMap<&str, String>, path: &str, value: &str) -> Result<(), SnapshotError> {
    if types.contains_key(value) {
        Ok(())
    } else {
        Err(SnapshotError::validation(
            path,
            format!("unknown type code name {value:?}"),
        ))
    }
}

fn unique_id<'a>(
    ids: &mut HashMap<&'a str, String>,
    path: &str,
    value: &'a str,
) -> Result<(), SnapshotError> {
    non_blank(path, value)?;
    if let Some(previous) = ids.insert(value, path.to_owned()) {
        Err(SnapshotError::validation(
            path,
            format!("duplicate registration ID; first declared at {previous}"),
        ))
    } else {
        Ok(())
    }
}

fn contiguous(path: &str, values: impl IntoIterator<Item = usize>) -> Result<(), SnapshotError> {
    for (expected, actual) in values.into_iter().enumerate() {
        if expected != actual {
            return Err(SnapshotError::validation(
                format!("{path}[{expected}]"),
                format!("expected order {expected}, found {actual}"),
            ));
        }
    }
    Ok(())
}

fn non_blank(path: &str, value: &str) -> Result<(), SnapshotError> {
    if value.trim().is_empty() {
        Err(SnapshotError::validation(path, "value must not be blank"))
    } else {
        Ok(())
    }
}

fn validate_sha(path: &str, value: &str) -> Result<(), SnapshotError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(SnapshotError::validation(
            path,
            "expected 64 lowercase hexadecimal characters",
        ))
    }
}

fn require_empty_when_unsupported<T>(
    path: &str,
    supported: bool,
    values: &[T],
) -> Result<(), SnapshotError> {
    if supported || values.is_empty() {
        Ok(())
    } else {
        Err(SnapshotError::validation(
            path,
            "capability is false but the file is not empty",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    static FIXTURE: OnceLock<(raw::Manifest, raw::Snapshot)> = OnceLock::new();

    fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
    }

    fn read<T: DeserializeOwned>(file: &str) -> T {
        serde_json::from_str(&fs::read_to_string(fixture_path().join(file)).unwrap()).unwrap()
    }

    fn fixture() -> (raw::Manifest, raw::Snapshot) {
        FIXTURE
            .get_or_init(|| {
                (
                    read("Manifest.json"),
                    raw::Snapshot {
                        aliases: read("Aliases.json"),
                        classes: read("ClassHierarchy.json"),
                        comparators: read("Comparators.json"),
                        conditions: read("Conditions.json"),
                        converters: read("Converters.json"),
                        differences: read("Differences.json"),
                        effects: read("Effects.json"),
                        event_values: read("EventValues.json"),
                        events: read("Events.json"),
                        expressions: read("Expressions.json"),
                        functions: read("Functions.json"),
                        operations: read("Operations.json"),
                        operators: read("Operators.json"),
                        plural_rules: read("PluralRules.json"),
                        properties: read("Properties.json"),
                        sections: read("Sections.json"),
                        structures: read("Structures.json"),
                        types: read("Types.json"),
                    },
                )
            })
            .clone()
    }

    fn assert_validation(error: SnapshotError, expected_path: &str, expected_message: &str) {
        match error {
            SnapshotError::Validation { path, message } => {
                assert_eq!(path, expected_path);
                assert!(
                    message.contains(expected_message),
                    "unexpected validation message: {message}"
                );
            }
            other => panic!("expected validation error, found {other:?}"),
        }
    }

    #[test]
    fn accepts_generated_modern_fixture() {
        let (manifest, data) = fixture();
        snapshot(&manifest, &data).unwrap();
    }

    #[test]
    fn rejects_duplicate_registration_ids() {
        let (manifest, mut data) = fixture();
        data.effects[0].registration_id = data.conditions[0].registration_id.clone();

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Effects.json[0].registrationId",
            "duplicate registration ID",
        );
    }

    #[test]
    fn rejects_duplicate_type_code_names() {
        let (manifest, mut data) = fixture();
        data.types[1].code_name = data.types[0].code_name.clone();

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Types.json[1].codeName",
            "duplicate type code name",
        );
    }

    #[test]
    fn rejects_unknown_class_references_with_the_source_path() {
        let (manifest, mut data) = fixture();
        data.conditions[0].element_class = "missing.Condition".to_owned();

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Conditions.json[0].elementClass",
            "unknown class",
        );
    }

    #[test]
    fn rejects_unknown_type_references_with_the_source_path() {
        let (manifest, mut data) = fixture();
        data.types[0].assignable_to = vec!["missing-type".to_owned()];

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Types.json[0].assignableTo[0]",
            "unknown type code name",
        );
    }

    #[test]
    fn rejects_data_for_an_unsupported_capability() {
        let (mut manifest, data) = fixture();
        manifest.capabilities.syntax_kinds.conditions = false;

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Conditions.json",
            "capability is false but the file is not empty",
        );
    }

    #[test]
    fn rejects_value_and_resolution_state_mismatches() {
        let (manifest, mut data) = fixture();
        data.conditions[0].supported_events = None;
        data.conditions[0].supported_events_state = Some(ResolutionState::Resolved);

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Conditions.json[0].supportedEvents",
            "value/state mismatch",
        );
    }

    #[test]
    fn rejects_non_contiguous_registration_order() {
        let (manifest, mut data) = fixture();
        data.conditions[0].registration_order = 1;

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Conditions.json[0]",
            "expected order 0, found 1",
        );
    }

    #[test]
    fn rejects_modern_215_event_values_missing_required_fields() {
        let (mut manifest, mut data) = fixture();
        manifest.capabilities.event_value_api = EventValueApi::Modern215;
        for value in &mut data.event_values {
            value.context_dependent = None;
        }
        data.event_values[0].accepted_changers = None;

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "EventValues.json[0]",
            "modern-2.15 requires patterns and acceptedChangers",
        );
    }

    #[test]
    fn rejects_event_value_times_outside_the_skript_range() {
        let (manifest, mut data) = fixture();
        data.event_values[0].time = 2;

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "EventValues.json[0].time",
            "expected -1, 0, or 1",
        );
    }

    #[test]
    fn rejects_alias_target_indices_out_of_range() {
        let (manifest, mut data) = fixture();
        let invalid_index = data.aliases.targets.len();
        data.aliases
            .aliases
            .insert("zz-invalid-target".to_owned(), invalid_index);

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Aliases.json.aliases.zz-invalid-target",
            "is out of range",
        );
    }

    #[test]
    fn rejects_unreferenced_alias_targets() {
        let (manifest, mut data) = fixture();
        let unreferenced_index = data.aliases.targets.len();
        data.aliases.targets.push(data.aliases.targets[0].clone());

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            &format!("Aliases.json.targets[{unreferenced_index}]"),
            "target is not referenced",
        );
    }

    #[test]
    fn rejects_missing_function_names() {
        let (manifest, mut data) = fixture();
        data.functions[0].name = None;

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Functions.json[0].name",
            "function name is required",
        );
    }

    #[test]
    fn rejects_blank_function_parameter_names() {
        let (manifest, mut data) = fixture();
        data.functions[0].parameters[0].name = "  ".to_owned();

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Functions.json[0].parameters[0].name",
            "value must not be blank",
        );
    }

    #[test]
    fn rejects_duplicate_function_signatures() {
        let (manifest, mut data) = fixture();
        data.functions[1].name = data.functions[0].name.clone();
        data.functions[1].parameters = data.functions[0].parameters.clone();

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Functions.json[1].name",
            "duplicate function signature",
        );
    }

    #[test]
    fn allows_function_overloads_with_different_parameter_types() {
        let (manifest, mut data) = fixture();
        assert_ne!(
            data.functions[0].parameters[0].parameter_type,
            data.functions[1].parameters[0].parameter_type
        );
        data.functions[1].name = data.functions[0].name.clone();

        snapshot(&manifest, &data).unwrap();
    }

    #[test]
    fn reports_the_exact_path_for_invalid_function_modifiers() {
        let (manifest, mut data) = fixture();
        let (function_index, parameter_index, modifier_index) = data
            .functions
            .iter()
            .enumerate()
            .find_map(|(function_index, function)| {
                function
                    .parameters
                    .iter()
                    .enumerate()
                    .find_map(|(parameter_index, parameter)| {
                        parameter
                            .modifiers
                            .iter()
                            .position(|modifier| {
                                matches!(modifier.kind, raw::ParameterModifierKind::Range)
                            })
                            .map(|modifier_index| (function_index, parameter_index, modifier_index))
                    })
            })
            .expect("modern fixture must contain a range modifier");
        data.functions[function_index].parameters[parameter_index].modifiers[modifier_index].min =
            None;

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            &format!(
                "Functions.json[{function_index}].parameters[{parameter_index}].modifiers[{modifier_index}]"
            ),
            "range modifiers require min and max",
        );
    }

    #[test]
    fn rejects_unknown_function_parameter_classes() {
        let (manifest, mut data) = fixture();
        data.functions[0].parameters[0].parameter_type = "missing.FunctionParameter".to_owned();

        assert_validation(
            snapshot(&manifest, &data).unwrap_err(),
            "Functions.json[0].parameters[0].type",
            "unknown class",
        );
    }
}
