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
        // BlockUtils removes ASCII spaces throughout states before Bukkit reads tokens.
        let normalized = body.replace(';', ",").replace(' ', "");
        let mut remaining = normalized.trim_start_matches(is_state_whitespace);
        while !remaining.is_empty() {
            let name = read_state_token(&mut remaining)?;
            remaining = remaining
                .trim_start_matches(is_state_whitespace)
                .strip_prefix('=')?;
            let value = read_state_token(&mut remaining)?;
            if name.is_empty() || value.is_empty() || properties.insert(name, value).is_some() {
                return None;
            }
            remaining = remaining.trim_start_matches(is_state_whitespace);
            if !remaining.is_empty() {
                remaining = remaining.strip_prefix(',')?;
            }
        }
    }
    Some(ParsedSyntax {
        base: base.to_owned(),
        properties,
        has_states: body.is_some(),
    })
}

// Brigadier uses Java Character.isWhitespace, which excludes nonbreaking spaces.
fn is_state_whitespace(character: char) -> bool {
    matches!(character,
        '\u{0009}'..='\u{000D}' | '\u{001C}'..='\u{0020}' | '\u{1680}' |
        '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200A}' |
        '\u{2028}'..='\u{2029}' | '\u{205F}' | '\u{3000}'
    )
}

fn read_state_token(input: &mut &str) -> Option<String> {
    let text = input.trim_start_matches(is_state_whitespace);
    let quote = text.chars().next()?;
    if matches!(quote, '\'' | '"') {
        let mut value = String::new();
        let mut escaped = false;
        for (offset, character) in text.char_indices().skip(1) {
            if escaped {
                if character != quote && character != '\\' {
                    return None;
                }
                value.push(character);
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                *input = &text[offset + character.len_utf8()..];
                return Some(value);
            } else {
                value.push(character);
            }
        }
        return None;
    }
    let end = text
        .find(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.' | '+')
        })
        .unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    *input = &text[end..];
    Some(text[..end].to_owned())
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
    let base = base.trim().replace(' ', "_");
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
                || normalize_id(&item.material.to_ascii_lowercase()),
                |id| {
                    normalize_id(
                        &id.split_once('[')
                            .map_or(id, |(base, _)| base)
                            .to_ascii_lowercase(),
                    )
                },
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
    use std::collections::{BTreeMap, BTreeSet};

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
        assert!(parse_syntax("chest[,]").is_none());
        assert!(parse_syntax("chest[facing=north,,]").is_none());
    }

    #[test]
    fn matches_bukkit_empty_trailing_delimiter_and_case_boundaries() {
        for input in [
            "chest[]",
            "chest[ ]",
            "chest[facing=north,]",
            "chest[facing=north;]",
            "chest[facing = north]",
        ] {
            let syntax = parse_syntax(input).unwrap();
            assert!(
                super::valid_properties(&chest(), &syntax.properties),
                "{input}"
            );
        }
        for input in [
            "chest[Facing=north]",
            "chest[facing=NORTH]",
            "chest[waterlogged=FALSE]",
        ] {
            let syntax = parse_syntax(input).unwrap();
            assert!(
                !super::valid_properties(&chest(), &syntax.properties),
                "{input}"
            );
        }
        assert_eq!(super::normalize_id("CHEST"), "minecraft:CHEST");
    }

    #[test]
    fn normalizes_internal_spaces_and_quoted_state_tokens() {
        for (input, expected) in [
            ("chest[fa cing=no rth]", "minecraft:chest[facing=north]"),
            (
                "chest[water logged=fa lse]",
                "minecraft:chest[waterlogged=false]",
            ),
            (r#"chest[facing="north"]"#, "minecraft:chest[facing=north]"),
            (r#"chest["facing"=north]"#, "minecraft:chest[facing=north]"),
            (
                r#"chest["facing"="north"]"#,
                "minecraft:chest[facing=north]",
            ),
            ("chest['facing'='north']", "minecraft:chest[facing=north]"),
            (
                r#"chest["fa cing"="no rth";]"#,
                "minecraft:chest[facing=north]",
            ),
            ("chest[facing=\tnorth]", "minecraft:chest[facing=north]"),
        ] {
            let syntax = parse_syntax(input).unwrap();
            assert!(
                super::valid_properties(&chest(), &syntax.properties),
                "{input}"
            );
            assert_eq!(
                canonical(&ParsedBlockData {
                    id: "minecraft:chest".to_owned(),
                    properties: syntax.properties
                }),
                expected,
                "{input}"
            );
        }
        for input in [
            r#"chest[facing="NORTH"]"#,
            r#"chest["Facing"="north"]"#,
            r#"chest[facing="nor\"th"]"#,
        ] {
            assert!(
                !super::valid_properties(&chest(), &parse_syntax(input).unwrap().properties),
                "{input}"
            );
        }
        for input in [
            r#"chest[facing="north"x]"#,
            r#"chest[facing="north]"#,
            r#"chest[facing="no\rth"]"#,
            r#"chest[facing=north;"facing"=south]"#,
            r#"chest[facing="north";;]"#,
            "chest[facing='north\"]",
            "chest[fa\tcing=north]",
        ] {
            assert!(parse_syntax(input).is_none(), "{input}");
        }
    }

    #[test]
    fn state_tokens_only_escape_the_matching_quote_or_backslash() {
        for (mut input, expected, rest) in [
            (r#""a\"b\\c",rest"#, r#"a"b\c"#, ",rest"),
            (r"'a\'b\\c',rest", r"a'b\c", ",rest"),
            (r#"""=rest"#, "", "=rest"),
        ] {
            assert_eq!(
                super::read_state_token(&mut input).as_deref(),
                Some(expected)
            );
            assert_eq!(input, rest);
        }
        for mut input in [r#""a\q""#, r#""a\""#, r#""unterminated"#] {
            assert!(super::read_state_token(&mut input).is_none());
        }
    }

    #[test]
    fn state_whitespace_and_token_positions_match_the_java_paper_matrix() {
        let cases = include_str!("../../tests/data/block-data-state-whitespace.tsv")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| {
                let fields = line.split('\t').collect::<Vec<_>>();
                let codepoint = u32::from_str_radix(fields[0], 16).unwrap();
                (
                    codepoint,
                    fields[1] == "true",
                    u32::from_str_radix(fields[2], 16).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let whitespace = cases
            .iter()
            .filter(|(_, whitespace, _)| *whitespace)
            .map(|(codepoint, _, _)| *codepoint)
            .collect::<BTreeSet<_>>();
        assert_eq!(whitespace.len(), 25);
        for codepoint in 0..=0x10FFFF {
            if let Some(character) = char::from_u32(codepoint) {
                assert_eq!(
                    super::is_state_whitespace(character),
                    whitespace.contains(&codepoint),
                    "U+{codepoint:04X}"
                );
            }
        }
        // Ordered exactly as the independent Paper probe that produced the masks.
        let placements = [
            ("chest[@facing=north]", "minecraft:chest[facing=north]"),
            ("chest[facing@=north]", "minecraft:chest[facing=north]"),
            (r#"chest["facing"@=north]"#, "minecraft:chest[facing=north]"),
            ("chest[facing=@north]", "minecraft:chest[facing=north]"),
            (r#"chest[facing=@"north"]"#, "minecraft:chest[facing=north]"),
            ("chest[facing=north@]", "minecraft:chest[facing=north]"),
            (r#"chest[facing="north"@]"#, "minecraft:chest[facing=north]"),
            (
                "chest[facing=north@,waterlogged=false]",
                "minecraft:chest[facing=north,waterlogged=false]",
            ),
            (
                "chest[facing=north,@waterlogged=false]",
                "minecraft:chest[facing=north,waterlogged=false]",
            ),
            ("chest[@]", "minecraft:chest"),
            ("chest[facing=north,@]", "minecraft:chest[facing=north]"),
            ("chest[fa@cing=north]", "minecraft:chest[facing=north]"),
            ("chest[facing=no@rth]", "minecraft:chest[facing=north]"),
            (r#"chest["fa@cing"=north]"#, "minecraft:chest[facing=north]"),
            (r#"chest[facing="no@rth"]"#, "minecraft:chest[facing=north]"),
            (r#"chest["@facing"=north]"#, "minecraft:chest[facing=north]"),
            (r#"chest["facing@"=north]"#, "minecraft:chest[facing=north]"),
            (r#"chest[facing="@north"]"#, "minecraft:chest[facing=north]"),
            (r#"chest[facing="north@"]"#, "minecraft:chest[facing=north]"),
            ("chest[facing='north'@]", "minecraft:chest[facing=north]"),
        ];
        assert_eq!(cases.len(), 302);
        for (codepoint, _, mask) in cases {
            let marker = char::from_u32(codepoint).unwrap().to_string();
            for (index, (template, expected)) in placements.iter().enumerate() {
                let input = template.replace('@', &marker);
                let actual = parse_syntax(&input).and_then(|syntax| {
                    super::valid_properties(&chest(), &syntax.properties).then(|| {
                        canonical(&ParsedBlockData {
                            id: "minecraft:chest".to_owned(),
                            properties: syntax.properties,
                        })
                    })
                });
                let expected = (mask & (1 << index) != 0).then_some(*expected);
                assert_eq!(
                    actual.as_deref(),
                    expected,
                    "U+{codepoint:04X} position={index}"
                );
            }
        }
    }
}
