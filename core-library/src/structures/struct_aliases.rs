use super::{append_metadata, continue_with_mode, direct_body_nodes, is_trivia, structure_error};
#[cfg(target_arch = "wasm32")]
use crate::nlaocs::skript_parser_addon::state_store;
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, RawNodeKind, RegisteredSyntaxHandler, StructureBodyMode,
    StructurePayload, StructureTiming,
};
#[cfg(target_arch = "wasm32")]
use crate::nlaocs::skript_parser_addon::types::{
    StateEncoding, StateNamespaceVisibility, StateScope, StateValue,
};
use std::collections::{HashMap, HashSet};

const CLASS_SUFFIX: &str = ".StructAliases";
const HANDLER_ID: &str = "core.structure.struct-aliases";
#[cfg(target_arch = "wasm32")]
const ALIAS_NAMESPACE: &str = "aliases";
#[cfg(target_arch = "wasm32")]
const ALIAS_SCHEMA: &str = "nlaocs.core-library.aliases";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    payload.candidate.handler.as_deref() == Some(HANDLER_ID)
        || crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: StructurePayload) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    let mut diagnostics = if entering {
        validate_body(&payload)
    } else {
        Vec::new()
    };
    if entering {
        append_metadata(&mut payload, "aliases-scope", "script");
        for alias in aliases(&payload) {
            if let Err(reason) = register_alias(&alias) {
                diagnostics.push(structure_error(
                    "core.struct-aliases.registry",
                    reason,
                    payload.candidate.span.clone(),
                ));
            }
        }
    }
    let mut output = continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Raw,
        "script-aliases",
        "core.structure.aliases",
    );
    output.effects.diagnostics.extend(diagnostics);
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScriptAlias {
    name: String,
    plural: bool,
    value: String,
}

fn aliases(payload: &StructurePayload) -> Vec<ScriptAlias> {
    let mut groups = HashMap::new();
    let mut aliases = Vec::new();

    // AliasesParser processes this section from top to bottom. A variation group
    // therefore becomes available only to aliases that appear after it.
    for node in direct_body_nodes(payload) {
        match node.kind {
            RawNodeKind::Simple => {
                aliases.extend(alias_entries(node.text.as_str(), &groups));
            }
            RawNodeKind::Section => {
                if let Some((name, variations)) = parse_variation_group(payload, node) {
                    groups.insert(name, variations);
                }
            }
            RawNodeKind::Blank | RawNodeKind::Comment | RawNodeKind::Invalid => {}
        }
    }

    aliases
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AliasVariation {
    name: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AliasExpansion {
    name: String,
    value: Option<String>,
}

fn alias_entries(
    name_and_value: &str,
    groups: &HashMap<String, Vec<AliasVariation>>,
) -> Vec<ScriptAlias> {
    let Some((name, value)) = name_and_value.split_once('=') else {
        return Vec::new();
    };
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return Vec::new();
    }

    parse_key_pattern(name)
        .into_iter()
        .flat_map(|pattern| expand_variation_references(&pattern, groups))
        .flat_map(|expansion| {
            let value = expansion.value.unwrap_or_else(|| value.to_owned());
            alias_forms(&expansion.name)
                .into_iter()
                .map(move |(name, plural)| ScriptAlias {
                    name,
                    plural,
                    value: value.clone(),
                })
        })
        .collect()
}

fn parse_variation_group(
    payload: &StructurePayload,
    node: &crate::nlaocs::skript_parser_addon::types::RawTreeNode,
) -> Option<(String, Vec<AliasVariation>)> {
    let name = node.text.trim();
    if !(name.starts_with('{') && name.ends_with('}') && name.len() > 2) {
        return None;
    }

    let mut variations: Vec<AliasVariation> = Vec::new();
    for child_id in &node.children {
        let Some(child) = payload
            .body_tree
            .nodes
            .iter()
            .find(|candidate| candidate.id == *child_id)
        else {
            continue;
        };
        if !matches!(child.kind, RawNodeKind::Simple) {
            continue;
        }
        let Some((key, value)) = child.text.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        for key in parse_key_pattern(key.trim()) {
            let key = if key == "{default}" {
                String::new()
            } else {
                key
            };
            if let Some(existing) = variations
                .iter_mut()
                .find(|variation| variation.name == key)
            {
                // VariationGroup.put stores the last value for a duplicate key
                // once parseKeyVariations materializes its LinkedHashMap.
                existing.value = value.to_owned();
            } else {
                variations.push(AliasVariation {
                    name: key,
                    value: value.to_owned(),
                });
            }
        }
    }

    Some((name.to_owned(), variations))
}

fn parse_key_pattern(name: &str) -> Vec<String> {
    let mut cursor = 0;
    let Ok((variants, end)) = parse_pattern_sequence(name, &mut cursor, None) else {
        return Vec::new();
    };
    if end != PatternEnd::End || cursor != name.len() {
        return Vec::new();
    }
    unique_strings(variants)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternEnd {
    End,
    Pipe,
    Close(u8),
}

fn parse_pattern_sequence(
    input: &str,
    cursor: &mut usize,
    closing: Option<u8>,
) -> Result<(Vec<String>, PatternEnd), ()> {
    let mut variants = vec![String::new()];

    while *cursor < input.len() {
        let byte = input.as_bytes()[*cursor];
        if closing == Some(byte) {
            *cursor += 1;
            return Ok((variants, PatternEnd::Close(byte)));
        }
        if byte == b'|' && closing == Some(b')') {
            *cursor += 1;
            return Ok((variants, PatternEnd::Pipe));
        }
        if byte == b']' || byte == b')' {
            return Err(());
        }

        let fragment = match byte {
            b'[' => {
                *cursor += 1;
                let (inner, end) = parse_pattern_sequence(input, cursor, Some(b']'))?;
                if end != PatternEnd::Close(b']') {
                    return Err(());
                }
                let mut optional = inner;
                optional.push(String::new());
                optional
            }
            b'(' => {
                *cursor += 1;
                let mut choices = Vec::new();
                loop {
                    let (choice, end) = parse_pattern_sequence(input, cursor, Some(b')'))?;
                    choices.extend(choice);
                    match end {
                        PatternEnd::Pipe => continue,
                        PatternEnd::Close(b')') => break,
                        _ => return Err(()),
                    }
                }
                choices
            }
            _ => {
                let start = *cursor;
                while *cursor < input.len()
                    && !matches!(input.as_bytes()[*cursor], b'[' | b']' | b'(' | b')')
                    && !(closing == Some(b')') && input.as_bytes()[*cursor] == b'|')
                {
                    *cursor += 1;
                }
                vec![input[start..*cursor].to_owned()]
            }
        };
        variants = concatenate_variants(variants, fragment);
    }

    if closing.is_some() {
        Err(())
    } else {
        Ok((variants, PatternEnd::End))
    }
}

fn concatenate_variants(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(left.len().saturating_mul(right.len()));
    for prefix in left {
        for suffix in &right {
            let mut value = prefix.clone();
            value.push_str(suffix);
            if !result.iter().any(|existing| existing == &value) {
                result.push(value);
            }
        }
    }
    result
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    values.into_iter().fold(Vec::new(), |mut unique, value| {
        if !unique.iter().any(|existing| existing == &value) {
            unique.push(value);
        }
        unique
    })
}

fn expand_variation_references(
    name: &str,
    groups: &HashMap<String, Vec<AliasVariation>>,
) -> Vec<AliasExpansion> {
    let mut expansions = vec![AliasExpansion {
        name: String::new(),
        value: None,
    }];
    let mut cursor = 0;

    while cursor < name.len() {
        let Some(relative_start) = name[cursor..].find('{') else {
            let suffix = &name[cursor..];
            for expansion in &mut expansions {
                expansion.name.push_str(suffix);
            }
            break;
        };
        let start = cursor + relative_start;
        let Some(relative_end) = name[start..].find('}') else {
            return Vec::new();
        };
        let end = start + relative_end + 1;
        let reference = &name[start..end];
        let Some(variations) = groups.get(reference) else {
            return Vec::new();
        };

        let prefix = &name[cursor..start];
        for expansion in &mut expansions {
            expansion.name.push_str(prefix);
        }
        let mut next = Vec::new();
        // AliasesParser's odometer increments the leftmost variation slot
        // first. Keep that order because later duplicate aliases overwrite
        // earlier ones in the provider's insertion map.
        for variation in variations {
            for expansion in &expansions {
                let mut next_expansion = expansion.clone();
                next_expansion.name.push_str(&variation.name);
                // Variation.merge lets the later variation ID replace the
                // base alias ID unless the base uses the advanced `-`
                // insertion syntax. Preserve the common ID replacement here;
                // tags, block states, and insertion remain opaque in `value`.
                next_expansion.value = Some(variation.value.clone());
                next.push(next_expansion);
            }
        }
        expansions = next;
        cursor = end;
    }

    expansions
}

fn alias_forms(name: &str) -> Vec<(String, bool)> {
    let name = name.rsplit_once('@').map_or(name, |(name, _)| name).trim();
    let Some(marker) = name.find('¦') else {
        return vec![(normalize_name(name), false)];
    };
    let plural_end = name[marker + '¦'.len_utf8()..]
        .find(char::is_whitespace)
        .map(|offset| marker + '¦'.len_utf8() + offset);
    let (singular, plural) = match plural_end {
        Some(plural_end) => {
            let base = &name[..marker];
            (
                format!("{base}{}", &name[plural_end..]),
                format!("{base}{}", &name[marker + '¦'.len_utf8()..]),
            )
        }
        None => {
            let singular = name[..marker].to_owned();
            let plural = format!("{}{}", singular, &name[marker + '¦'.len_utf8()..]);
            (singular, plural)
        }
    };
    let singular = normalize_name(&singular);
    let plural = normalize_name(&plural);
    if singular == plural {
        vec![(singular, false)]
    } else {
        vec![(singular, false), (plural, true)]
    }
}

fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

#[cfg(target_arch = "wasm32")]
fn register_alias(alias: &ScriptAlias) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(alias.value.len() + 1);
    bytes.push(u8::from(alias.plural));
    bytes.extend_from_slice(alias.value.as_bytes());
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        ALIAS_NAMESPACE,
        &alias.name,
        &StateValue {
            schema_id: ALIAS_SCHEMA.to_owned(),
            encoding: StateEncoding::Raw,
            bytes,
        },
    )
    .map_err(|error| {
        format!(
            "failed to register script alias `{}`: {}",
            alias.name, error.message
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn register_alias(_alias: &ScriptAlias) -> Result<(), String> {
    Ok(())
}

fn validate_body(
    payload: &StructurePayload,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let mut groups = HashSet::new();
    let mut diagnostics = Vec::new();
    for node in direct_body_nodes(payload) {
        if is_trivia(node) {
            continue;
        }
        diagnostics.extend(validate_node(payload, node));
        match node.kind {
            RawNodeKind::Simple => {
                let name = node
                    .text
                    .split_once('=')
                    .map(|(name, _)| name)
                    .unwrap_or("");
                for reference in variation_references(name) {
                    if !groups.contains(reference) {
                        diagnostics.push(structure_error(
                            "core.struct-aliases.unknown-variation-group",
                            format!("alias references unknown variation group `{reference}`"),
                            node.span.clone(),
                        ));
                    }
                }
            }
            RawNodeKind::Section => {
                let name = node.text.trim();
                if name.starts_with('{') && name.ends_with('}') && name.len() > 2 {
                    groups.insert(name.to_owned());
                }
            }
            RawNodeKind::Blank | RawNodeKind::Comment | RawNodeKind::Invalid => {}
        }
    }
    diagnostics
}

fn variation_references(name: &str) -> Vec<&str> {
    let mut references = Vec::new();
    let mut remaining = name;
    while let Some(start) = remaining.find('{') {
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        references.push(&remaining[..=end]);
        remaining = &remaining[end + 1..];
    }
    references
}

fn validate_node(
    payload: &StructurePayload,
    node: &crate::nlaocs::skript_parser_addon::types::RawTreeNode,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    match node.kind {
        RawNodeKind::Simple => validate_alias_line(node),
        RawNodeKind::Section => validate_variation_group(payload, node),
        RawNodeKind::Invalid => vec![structure_error(
            "core.struct-aliases.invalid-entry",
            "this aliases entry is not a valid Skript source line",
            node.span.clone(),
        )],
        RawNodeKind::Blank | RawNodeKind::Comment => Vec::new(),
    }
}

fn validate_variation_group(
    payload: &StructurePayload,
    node: &crate::nlaocs::skript_parser_addon::types::RawTreeNode,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let name = node.text.trim();
    let mut diagnostics = Vec::new();
    if !(name.starts_with('{') && name.ends_with('}') && name.len() > 2) {
        diagnostics.push(structure_error(
            "core.struct-aliases.invalid-variation-group",
            "an aliases subsection must be a named variation group such as `{wood}`",
            node.span.clone(),
        ));
    }
    for child_id in &node.children {
        let Some(child) = payload
            .body_tree
            .nodes
            .iter()
            .find(|candidate| candidate.id == *child_id)
        else {
            continue;
        };
        if is_trivia(child) {
            continue;
        }
        match child.kind {
            RawNodeKind::Simple => diagnostics.extend(validate_alias_line(child)),
            _ => diagnostics.push(structure_error(
                "core.struct-aliases.invalid-variation",
                "an alias variation must be a simple `name = value` entry",
                child.span.clone(),
            )),
        }
    }
    diagnostics
}

fn validate_alias_line(
    node: &crate::nlaocs::skript_parser_addon::types::RawTreeNode,
) -> Vec<crate::nlaocs::skript_parser_addon::types::Diagnostic> {
    let Some((name, values)) = node.text.split_once('=') else {
        return vec![structure_error(
            "core.struct-aliases.missing-separator",
            "an aliases entry must contain `=` between its name and values",
            node.span.clone(),
        )];
    };
    let mut diagnostics = Vec::new();
    if name.trim().is_empty() {
        diagnostics.push(structure_error(
            "core.struct-aliases.empty-name",
            "an aliases entry must have a non-empty name",
            node.span.clone(),
        ));
    }
    if values.trim().is_empty() {
        diagnostics.push(structure_error(
            "core.struct-aliases.empty-values",
            "an aliases entry must have at least one value",
            node.span.clone(),
        ));
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        AliasVariation, alias_entries, alias_forms, expand_variation_references, parse_key_pattern,
        variation_references,
    };

    #[test]
    fn alias_plural_marker_matches_skripts_forms() {
        assert_eq!(
            alias_forms("shiny sword¦s"),
            [
                ("shiny sword".to_owned(), false),
                ("shiny swords".to_owned(), true)
            ]
        );
        assert_eq!(
            alias_forms("red ¦wool block"),
            [
                ("red block".to_owned(), false),
                ("red wool block".to_owned(), true)
            ]
        );
    }

    #[test]
    fn alias_gender_suffix_is_not_part_of_the_name() {
        assert_eq!(alias_forms("stone @neuter"), [("stone".to_owned(), false)]);
    }

    #[test]
    fn variation_references_are_reported_in_source_order() {
        assert_eq!(
            variation_references("{wood} {shape} block"),
            ["{wood}", "{shape}"]
        );
        assert!(variation_references("plain block").is_empty());
    }

    #[test]
    fn alias_key_patterns_expand_optionals_and_choices() {
        assert_eq!(
            parse_key_pattern("cobble[stone]"),
            ["cobblestone".to_owned(), "cobble".to_owned()]
        );
        assert_eq!(
            parse_key_pattern("(oak|spruce)"),
            ["oak".to_owned(), "spruce".to_owned()]
        );
        // AliasesParser treats `|` as a choice separator only inside `()`;
        // elsewhere it is ordinary alias text.
        assert_eq!(parse_key_pattern("oak|spruce"), ["oak|spruce".to_owned()]);
        assert_eq!(
            parse_key_pattern("[oak|spruce]"),
            ["oak|spruce".to_owned(), "".to_owned()]
        );

        let aliases = alias_entries("[cool ](stone|rock) = stone", &HashMap::new());
        assert_eq!(
            aliases
                .into_iter()
                .map(|alias| alias.name)
                .collect::<Vec<_>>(),
            [
                "cool stone".to_owned(),
                "cool rock".to_owned(),
                "stone".to_owned(),
                "rock".to_owned(),
            ]
        );
    }

    #[test]
    fn variation_references_expand_as_a_cartesian_product() {
        let groups = HashMap::from([
            (
                "{wood}".to_owned(),
                vec![
                    AliasVariation {
                        name: "oak".to_owned(),
                        value: "oak".to_owned(),
                    },
                    AliasVariation {
                        name: "birch".to_owned(),
                        value: "birch".to_owned(),
                    },
                ],
            ),
            (
                "{shape}".to_owned(),
                vec![
                    AliasVariation {
                        name: "plank".to_owned(),
                        value: "plank".to_owned(),
                    },
                    AliasVariation {
                        name: "log".to_owned(),
                        value: "log".to_owned(),
                    },
                ],
            ),
        ]);

        let expansions = expand_variation_references("{wood} {shape}", &groups);
        assert_eq!(
            expansions
                .iter()
                .map(|expansion| expansion.name.as_str())
                .collect::<Vec<_>>(),
            ["oak plank", "birch plank", "oak log", "birch log"]
        );
        assert_eq!(
            expansions
                .iter()
                .map(|expansion| expansion.value.as_deref())
                .collect::<Vec<_>>(),
            [Some("plank"), Some("plank"), Some("log"), Some("log")]
        );

        let aliases = alias_entries("{wood} {shape} = fallback", &groups);
        assert_eq!(
            aliases
                .iter()
                .map(|alias| alias.value.as_str())
                .collect::<Vec<_>>(),
            ["plank", "plank", "log", "log"]
        );
    }
}
