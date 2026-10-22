//! Definition-backed entry keys from the current owner's frozen native result.

#[cfg(test)]
mod tests {
    use super::*;
    fn declaration() -> EntryData {
        EntryData {
            key: "value".into(),
            default_value: None,
            optional: false,
            multiple: true,
            entry_data_class: syntaxes::ClassName("test.Entry".into()),
            kind: EntryKind::Expression,
            separator: Some(": ".into()),
            value_type: None,
            string_mode: None,
            return_types: vec![syntaxes::ClassName("java.lang.Number".into())],
            flags: Some(3),
            nested_validator: None,
        }
    }
    fn entry(data: &EntryData, index: usize) -> StructureEntry {
        let source = skript_parser::MappedSource::identity("value: 1");
        let tree = skript_parser::parse_raw_tree(
            &source,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        );
        let range = TextRange::new(0, data.key.len());
        let span = skript_parser::MatchSpan {
            local_range: range,
            mapped: source.map_range(range).unwrap(),
        };
        StructureEntry {
            raw_node_id: Some(tree.roots[0]),
            declaration_index: index,
            key_span: Some(span.clone()),
            key: data.key.clone(),
            entry_data_class: data.entry_data_class.clone(),
            kind: data.kind.clone(),
            source: "value: 1".into(),
            span,
            defaulted: false,
            value: StructureEntryValue::Raw("1".into()),
        }
    }
    #[test]
    fn selected_declaration_controls_hover_and_unknown_multiplicity_is_honest() {
        let second = declaration();
        let mut first = second.clone();
        first.optional = true;
        first.return_types.clear();
        let validator = EntryValidator {
            entry_data: vec![first, second.clone()],
        };
        let mut targets = Vec::new();
        let mut blocked = Vec::new();
        collect(
            &[entry(&second, 1)],
            &validator,
            &[],
            &BTreeMap::new(),
            "owner",
            &MarkupKind::PlainText,
            None,
            &mut targets,
            &mut blocked,
        )
        .unwrap();
        assert_eq!(targets.len(), 1);
        let text = &targets[0].content;
        assert!(text.contains("Requirement: required"));
        assert!(text.contains("Entry occurrences: repeatable"));
        assert!(text.contains("Value type: java.lang.Number"));
        assert!(text.contains("Value multiplicity: unknown (not declared)"));
        assert_eq!(targets[0].origins, [TextRange::new(0, 5)]);
        assert!(blocked.is_empty());
    }
    #[test]
    fn implicit_defaults_non_exact_keys_and_invalid_declaration_identity_are_not_hovered() {
        let data = declaration();
        let validator = EntryValidator {
            entry_data: vec![data.clone()],
        };
        let mut implicit = entry(&data, 0);
        implicit.raw_node_id = None;
        implicit.key_span = None;
        implicit.defaulted = true;
        let mut anchored = entry(&data, 0);
        anchored.key_span.as_mut().unwrap().mapped.origins[0].kind =
            skript_parser::OriginKind::Anchored;
        let mut targets = Vec::new();
        let mut blocked = Vec::new();
        collect(
            &[implicit, anchored],
            &validator,
            &[],
            &BTreeMap::new(),
            "owner",
            &MarkupKind::Markdown,
            None,
            &mut targets,
            &mut blocked,
        )
        .unwrap();
        assert!(targets.is_empty());
        assert_eq!(blocked, [TextRange::new(0, 5)]);
        let mut invalid = entry(&data, 0);
        invalid.key = "other".into();
        assert!(
            collect(
                &[invalid],
                &validator,
                &[],
                &BTreeMap::new(),
                "owner",
                &MarkupKind::PlainText,
                None,
                &mut targets,
                &mut blocked
            )
            .is_err()
        );
    }
    #[test]
    fn section_body_kind_comes_from_owner_and_unknown_value_type_stays_unknown() {
        let mut data = declaration();
        data.kind = EntryKind::Section;
        let path = vec!["nested".into(), "value".into()];
        let text = describe(
            &data,
            &path,
            Some(&SectionBodyMode::Conditions),
            "owner",
            0,
            &MarkupKind::PlainText,
            None,
        );
        assert!(text.contains("Entry path: nested / value"));
        assert!(text.contains("Shape: section"));
        assert!(text.contains("Body: conditions"));
        let text = describe(&data, &path, None, "owner", 0, &MarkupKind::PlainText, None);
        assert!(text.contains("raw (no executable body declaration)"));
        data.kind = EntryKind::Unknown;
        data.return_types.clear();
        let text = describe(&data, &path, None, "owner", 0, &MarkupKind::PlainText, None);
        assert!(text.contains("Value type: unknown (not declared)"));
        assert!(text.contains("Shape: unknown (provider-defined)"));
    }
}

use super::*;
use skript_parser::{
    SectionBodyMode, StructureBody, StructureDocumentNode, StructureEntry, StructureEntryValue,
};
use syntaxes::{EntryData, EntryKind, EntryValidator};

pub(super) fn targets(
    result: &DocumentParseResult,
    format: &MarkupKind,
    catalog: &Catalog,
) -> Result<(Vec<Target>, Vec<TextRange>), String> {
    let mut targets = Vec::new();
    let mut blocked = Vec::new();
    for root in &result.syntax.roots {
        let StructureDocumentNode::Structure(matches) = root else {
            continue;
        };
        let candidate = matches.selected.as_ref().or_else(|| {
            matches
                .unknown
                .as_ref()?
                .partial
                .as_deref()
                .filter(|candidate| candidate.entry_validation_failed)
        });
        let Some(candidate) = candidate else { continue };
        if let (Some(validator), StructureBody::Entries(entries)) =
            (&candidate.entry_validator, &candidate.body)
        {
            collect(
                entries,
                validator,
                &[],
                &candidate.entry_body_modes,
                &candidate.matched.registration_id,
                format,
                Some(catalog),
                &mut targets,
                &mut blocked,
            )?;
        }
    }
    Ok((targets, blocked))
}

fn collect(
    entries: &[StructureEntry],
    validator: &EntryValidator,
    path: &[String],
    modes: &BTreeMap<Vec<String>, SectionBodyMode>,
    registration: &str,
    format: &MarkupKind,
    catalog: Option<&Catalog>,
    targets: &mut Vec<Target>,
    blocked: &mut Vec<TextRange>,
) -> Result<(), String> {
    for entry in entries {
        let data = validator
            .entry_data
            .get(entry.declaration_index)
            .ok_or("entry declaration index is invalid")?;
        if data.key != entry.key
            || data.kind != entry.kind
            || data.entry_data_class != entry.entry_data_class
        {
            return Err("entry declaration identity differs from its frozen validator".into());
        }
        let mut path = path.to_vec();
        path.push(data.key.clone());
        if let Some(raw) = entry.raw_node_id {
            if let Some(span) = &entry.key_span {
                let exact = span
                    .mapped
                    .origins
                    .iter()
                    .all(|origin| origin.kind == skript_parser::OriginKind::Exact);
                if exact && !span.mapped.origins.is_empty() {
                    targets.push(Target {
                        id: TargetId::Entry(raw),
                        depth: usize::MAX,
                        content: describe(
                            data,
                            &path,
                            modes.get(&path),
                            registration,
                            entry.declaration_index,
                            format,
                            catalog,
                        )
                        .into(),
                        origins: span
                            .mapped
                            .origins
                            .iter()
                            .map(|origin| origin.original_range)
                            .collect(),
                    });
                } else {
                    blocked.extend(
                        span.mapped
                            .origins
                            .iter()
                            .map(|origin| origin.original_range),
                    );
                }
            } else {
                blocked.extend(
                    entry
                        .span
                        .mapped
                        .origins
                        .iter()
                        .map(|origin| origin.original_range),
                );
            }
        }
        if let (StructureEntryValue::Container(children), Some(nested)) =
            (&entry.value, &data.nested_validator)
        {
            collect(
                children,
                nested,
                &path,
                modes,
                registration,
                format,
                catalog,
                targets,
                blocked,
            )?;
        }
    }
    Ok(())
}

fn describe(
    data: &EntryData,
    path: &[String],
    mode: Option<&SectionBodyMode>,
    registration: &str,
    index: usize,
    format: &MarkupKind,
    catalog: Option<&Catalog>,
) -> String {
    let type_name = |class: &str| {
        catalog.map_or_else(
            || class.to_owned(),
            |catalog| super::super::hover_text::type_name(catalog, class),
        )
    };
    let section = matches!(
        data.kind,
        EntryKind::Trigger | EntryKind::Container | EntryKind::Section
    );
    let mut fields = vec![
        ("Entry path", path.join(" / ")),
        ("Kind", format!("{:?}", data.kind)),
        (
            "Shape",
            if section {
                "section"
            } else if data.kind == EntryKind::Unknown {
                "unknown (provider-defined)"
            } else {
                "value"
            }
            .into(),
        ),
        (
            "Requirement",
            if data.optional {
                "optional"
            } else {
                "required"
            }
            .into(),
        ),
        (
            "Entry occurrences",
            if data.multiple {
                "repeatable"
            } else {
                "at most once"
            }
            .into(),
        ),
    ];
    if section {
        let body = match mode {
            Some(SectionBodyMode::Conditions) => "conditions",
            Some(SectionBodyMode::Trigger) => "executable statements",
            _ if data.kind == EntryKind::Trigger => "executable statements",
            _ if data.kind == EntryKind::Container => "nested entries",
            _ => "raw (no executable body declaration)",
        };
        fields.push(("Body", body.into()));
    } else {
        let value_type = data
            .value_type
            .as_ref()
            .map(|class| type_name(class.as_str()))
            .or_else(|| {
                (!data.return_types.is_empty()).then(|| {
                    data.return_types
                        .iter()
                        .map(|class| type_name(class.as_str()))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
            })
            .unwrap_or_else(|| "unknown (not declared)".into());
        fields.push(("Value type", value_type));
        let classes = data
            .value_type
            .iter()
            .chain(data.return_types.iter())
            .map(|class| class.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(" | ");
        if !classes.is_empty() {
            fields.push(("Value class", classes));
        }
        fields.push(("Value multiplicity", "unknown (not declared)".into()));
        fields.push((
            "Separator",
            data.separator.as_deref().unwrap_or(": ").into(),
        ));
    }
    if let Some(default) = &data.default_value {
        fields.push(("Default", default.to_string()));
    }
    if let Some(mode) = &data.string_mode {
        fields.push(("String mode", mode.clone()));
    }
    if let Some(flags) = data.flags {
        fields.push(("Parse flags", flags.to_string()));
    }
    fields.push(("Entry class", data.entry_data_class.0.clone()));
    fields.push(("Structure registration", registration.into()));
    fields.push(("Declaration index", index.to_string()));
    super::super::hover_text::render(
        &format!("Structure entry: {}", data.key),
        &fields,
        None,
        format,
    )
}
