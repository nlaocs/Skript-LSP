use crate::catalog::{AliasMaterial, BlockDataEntry, BlockDataRegistryState, BlockDataStatus};
use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
    TypeParserUnresolved,
};
use std::collections::{BTreeMap, BTreeSet};

const BLOCK_DATA_CLASS: &str = "org.bukkit.block.data.BlockData";
const REQUIRED_PROVIDER: &str = "ssg.block-data-registry";
const ALIAS_PROVIDER: &str = "ssg.aliases";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.block-data",
    classes: &[BLOCK_DATA_CLASS],
    parse,
    unresolved: Some(unresolved),
    all_type_options: false,
};

#[derive(Debug, PartialEq, Eq)]
struct ParsedBlockData {
    id: String,
    properties: BTreeMap<String, String>,
}

struct ResolvedBlockData {
    parsed: ParsedBlockData,
    definition: BlockDataEntry,
}

struct ResolveFailure {
    reason: String,
    provider: &'static str,
}

fn parse(payload: &ExpressionPayload, text: &str, end: u64) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let status = registry_status().ok()?;
    if status.state != BlockDataRegistryState::Collected {
        return None;
    }
    let resolved = parse_input(text).ok().flatten()?;
    if !valid_properties(&resolved.definition, &resolved.parsed.properties) {
        return None;
    }

    let canonical = canonical(&resolved.parsed);
    let mut result = candidate(
        PARSER.id,
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        BLOCK_DATA_CLASS,
        DynamicMultiplicity::Single,
    );
    result.metadata.extend([
        metadata("literal-canonical", &canonical),
        metadata("literal-source", "type-parser"),
        metadata("block-data-id", &resolved.parsed.id),
        metadata(
            "block-data-default-state",
            &resolved.definition.default_state,
        ),
        metadata(
            "block-data-registry-provider",
            status.registry_provider.as_deref().unwrap_or("unresolved"),
        ),
        metadata(
            "block-data-properties",
            &serde_json::to_string(&resolved.parsed.properties).ok()?,
        ),
    ]);
    Some(result)
}

fn unresolved(_payload: &ExpressionPayload, text: &str) -> Option<TypeParserUnresolved> {
    let status = match registry_status() {
        Ok(status) => status,
        Err(reason) => return Some(unresolved_reason(reason, REQUIRED_PROVIDER)),
    };
    match status.state {
        BlockDataRegistryState::Unsupported => {
            return Some(unresolved_reason(
                "the target Minecraft runtime does not expose Bukkit BlockData",
                REQUIRED_PROVIDER,
            ));
        }
        BlockDataRegistryState::Unresolved => {
            return Some(unresolved_reason(
                status.first_failure.unwrap_or_else(|| {
                    "the BlockData runtime registry could not be collected".to_owned()
                }),
                REQUIRED_PROVIDER,
            ));
        }
        BlockDataRegistryState::Collected => {}
    }

    let parsed = parse_syntax(text)?;
    match resolve_block(&parsed.base, parsed.has_states) {
        Ok(Some(_)) => {}
        Ok(None) if !status.complete => {
            return Some(unresolved_reason(
                format!(
                    "the incomplete BlockData registry does not contain {}",
                    parsed.base
                ),
                REQUIRED_PROVIDER,
            ));
        }
        Ok(None) if parsed.has_states && aliases_unresolved() => {
            return Some(unresolved_reason(
                "the SSG snapshot did not collect the global aliases needed by BlockData",
                ALIAS_PROVIDER,
            ));
        }
        Err(failure) => {
            return Some(unresolved_reason(failure.reason, failure.provider));
        }
        Ok(None) => {}
    }
    None
}

fn unresolved_reason(reason: impl Into<String>, provider: &str) -> TypeParserUnresolved {
    TypeParserUnresolved {
        reason: reason.into(),
        required_provider: Some(provider.to_owned()),
    }
}

fn aliases_unresolved() -> bool {
    crate::runtime::current()
        .and_then(|profile| profile.snapshot_capabilities)
        .is_some_and(|capabilities| {
            capabilities.aliases.supported && !capabilities.aliases.collected
        })
}

fn registry_status() -> Result<BlockDataStatus, String> {
    crate::catalog::block_data_status()?
        .ok_or_else(|| "the SSG snapshot does not contain BlockData.json".to_owned())
}

struct ParsedSyntax {
    base: String,
    properties: BTreeMap<String, String>,
    has_states: bool,
}

fn parse_input(text: &str) -> Result<Option<ResolvedBlockData>, ResolveFailure> {
    let Some(syntax) = parse_syntax(text) else {
        return Ok(None);
    };
    let Some((id, definition)) = resolve_block(&syntax.base, syntax.has_states)? else {
        return Ok(None);
    };
    Ok(Some(ResolvedBlockData {
        parsed: ParsedBlockData {
            id,
            properties: syntax.properties,
        },
        definition,
    }))
}

fn parse_syntax(text: &str) -> Option<ParsedSyntax> {
    let text = text.trim();
    let (base, body) = match text.find('[') {
        Some(start) => {
            if !text.ends_with(']') || text[start + 1..text.len() - 1].contains(['[', ']']) {
                return None;
            }
            (&text[..start], Some(&text[start + 1..text.len() - 1]))
        }
        None if text.contains(']') => return None,
        None => (text, None),
    };
    let base = base.trim();
    if base.is_empty() {
        return None;
    }

    let mut properties = BTreeMap::new();
    if let Some(body) = body {
        if body.trim().is_empty() {
            return None;
        }
        for assignment in body.split([',', ';']) {
            let (name, value) = assignment.split_once('=')?;
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_ascii_lowercase();
            if name.is_empty() || value.is_empty() || properties.insert(name, value).is_some() {
                return None;
            }
        }
    }
    Some(ParsedSyntax {
        base: base.to_owned(),
        properties,
        has_states: body.is_some(),
    })
}

fn resolve_block(
    base: &str,
    has_states: bool,
) -> Result<Option<(String, BlockDataEntry)>, ResolveFailure> {
    let normalized = normalize_id(base);
    if let Some(block) = crate::catalog::block_data(&normalized).map_err(block_data_failure)? {
        return Ok(Some((normalized, block)));
    }
    if !has_states {
        return Ok(None);
    }
    resolve_alias(base)
}

fn normalize_id(base: &str) -> String {
    let base = base.trim().replace(' ', "_").to_ascii_lowercase();
    if base.contains(':') {
        base
    } else {
        format!("minecraft:{base}")
    }
}

fn resolve_alias(base: &str) -> Result<Option<(String, BlockDataEntry)>, ResolveFailure> {
    let raw = base.trim().to_ascii_lowercase();
    let without_article = crate::language::strip_indefinite_article(&raw);
    let mut materials = crate::catalog::alias_materials(&raw).map_err(alias_failure)?;
    if materials.is_none() && without_article != raw {
        materials = crate::catalog::alias_materials(without_article).map_err(alias_failure)?;
    }
    let Some(materials) = materials else {
        return Ok(None);
    };

    let mut matches = Vec::new();
    for id in alias_block_ids(&materials) {
        if let Some(block) = crate::catalog::block_data(&id).map_err(block_data_failure)? {
            matches.push((id, block));
        }
    }
    Ok((matches.len() == 1).then(|| matches.pop()).flatten())
}

fn alias_block_ids(materials: &[AliasMaterial]) -> BTreeSet<String> {
    materials
        .iter()
        .map(|item| {
            item.minecraft_id.as_deref().map_or_else(
                || normalize_id(&item.material),
                |id| normalize_id(id.split_once('[').map_or(id, |(base, _)| base)),
            )
        })
        .collect()
}

fn block_data_failure(reason: String) -> ResolveFailure {
    ResolveFailure {
        reason,
        provider: REQUIRED_PROVIDER,
    }
}

fn alias_failure(reason: String) -> ResolveFailure {
    ResolveFailure {
        reason,
        provider: ALIAS_PROVIDER,
    }
}

fn valid_properties(block: &BlockDataEntry, properties: &BTreeMap<String, String>) -> bool {
    properties.iter().all(|(name, value)| {
        block
            .properties
            .get(name)
            .is_some_and(|values| values.contains(value))
    })
}

fn canonical(value: &ParsedBlockData) -> String {
    if value.properties.is_empty() {
        return value.id.clone();
    }
    let states = value
        .properties
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{}[{states}]", value.id)
}

#[cfg(test)]
mod tests {
    use super::{AliasMaterial, BlockDataEntry, ParsedBlockData, canonical, parse_syntax};
    use std::collections::BTreeMap;

    fn chest() -> BlockDataEntry {
        BlockDataEntry {
            default_state: "minecraft:chest[facing=north,waterlogged=false]".to_owned(),
            properties: BTreeMap::from([
                (
                    "facing".to_owned(),
                    ["east", "north", "south", "west"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                (
                    "waterlogged".to_owned(),
                    ["false", "true"].into_iter().map(str::to_owned).collect(),
                ),
            ]),
        }
    }

    #[test]
    fn validates_runtime_properties_without_requiring_sorted_values() {
        let syntax = parse_syntax("chest [ waterlogged = true; facing = west ]").unwrap();
        let parsed = ParsedBlockData {
            id: "minecraft:chest".to_owned(),
            properties: syntax.properties,
        };

        assert_eq!(
            canonical(&parsed),
            "minecraft:chest[facing=west,waterlogged=true]"
        );
        assert!(super::valid_properties(&chest(), &parsed.properties));

        let invalid = parse_syntax("chest[facing=sideways]").unwrap();
        assert!(!super::valid_properties(&chest(), &invalid.properties));
    }

    #[test]
    fn normalizes_alias_material_ids_and_removes_embedded_states() {
        let materials = [AliasMaterial {
            material: "RED_WOOL".to_owned(),
            minecraft_id: Some("minecraft:red_wool[waterlogged=false]".to_owned()),
        }];

        assert_eq!(
            super::alias_block_ids(&materials),
            ["minecraft:red_wool".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn rejects_duplicate_states_and_broken_brackets() {
        assert!(parse_syntax("chest[unknown=north]").is_some());
        assert!(parse_syntax("chest[facing=north,facing=south]").is_none());
        assert!(parse_syntax("chest[facing=north").is_none());
        assert!(parse_syntax("chest[]").is_none());
    }
}
