use parser_wasm::host::AstNode;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VariableIdentity {
    pub name: String,
    pub scope: String,
    pub resolution: String,
    pub alias_key: Option<String>,
    pub list: bool,
    #[serde(default)]
    pub name_range: Option<VariableNameRange>,
}

#[derive(Deserialize)]
pub(super) struct VariableNameRange {
    pub start: u64,
    pub end: u64,
    #[serde(default)]
    pub grammar: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParameterBindings {
    pub version: u32,
    pub parameters: Vec<ParameterBinding>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ParameterBinding {
    pub identity: VariableIdentity,
    pub class_name: String,
    pub single: bool,
    pub start: u64,
    pub end: u64,
}

/// Shared provider contract used by semantic facts and current-header completion.
/// Repeated or unsupported records cannot certify parameter declarations.
pub(super) fn parameter_bindings<'a>(
    metadata: impl Iterator<Item = (&'a str, &'a str)>,
) -> Option<ParameterBindings> {
    let mut values =
        metadata.filter_map(|(key, value)| (key == "skript.semantic.parameters").then_some(value));
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let data: ParameterBindings = serde_json::from_str(value).ok()?;
    matches!(data.version, 1 | 2).then_some(data)
}

/// Meaning is supplied by the semantic provider; geometry comes only from the
/// native capture seal. Unknown or ambiguous declarations add no coloring.
pub(super) fn expression_capture_roles(node: &AstNode) -> Option<Vec<u64>> {
    let mut records = node
        .summary
        .as_ref()?
        .public_data
        .iter()
        .filter(|record| record.schema_id == "nlaocs.skript.capture-roles");
    let record = records.next()?;
    if records.next().is_some() || record.schema_version != 1 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&record.json).ok()?;
    let entries = value.get("captures")?.as_array()?;
    if entries.len() > node.captures.len() {
        return None;
    }
    let mut indices = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let index = entry.get("captureIndex")?.as_u64()?;
        if entry.get("role")?.as_str()? != "expression" || !seen.insert(index) {
            return None;
        }
        indices.push(index);
    }
    Some(indices)
}

pub(super) struct InterpolationPart {
    pub child_index: usize,
    pub opening: usize,
    pub closing: usize,
}

/// Read only the parser-confirmed v1 contract. A child can trim surrounding
/// whitespace, so its immediate neighbours do not identify delimiters.
pub(super) fn interpolation_parts(node: &AstNode) -> Option<Vec<InterpolationPart>> {
    let mut records = node
        .summary
        .as_ref()?
        .public_data
        .iter()
        .filter(|record| record.schema_id == "nlaocs.skript.interpolation");
    let record = records.next()?;
    if records.next().is_some() || record.schema_version != 1 {
        return None;
    }
    let data: serde_json::Value = serde_json::from_str(&record.json).ok()?;
    let values = data.get("expressions")?.as_array()?;
    if values.len() != node.children.len() {
        return None;
    }
    let mut parts = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let child_index = usize::try_from(value.get("childIndex")?.as_u64()?).ok()?;
        let opening = usize::try_from(value.get("opening")?.as_u64()?).ok()?;
        let closing = usize::try_from(value.get("closing")?.as_u64()?).ok()?;
        if child_index != index
            || opening >= closing
            || closing >= node.text.len()
            || node.text.as_bytes().get(opening) != Some(&b'%')
            || node.text.as_bytes().get(closing) != Some(&b'%')
            || parts
                .last()
                .is_some_and(|previous: &InterpolationPart| previous.closing >= opening)
        {
            return None;
        }
        parts.push(InterpolationPart {
            child_index,
            opening,
            closing,
        });
    }
    Some(parts)
}

/// Read the versioned public name/scope contract without inferring a value,
/// fixed type, declaration binding, or storage policy from the source spelling.
pub(super) fn variable_details(node: &AstNode) -> Option<(String, String)> {
    let mut records = node
        .summary
        .as_ref()?
        .public_data
        .iter()
        .filter(|data| data.schema_id == "nlaocs.skript.variable");
    let data = records.next()?;
    if records.next().is_some() || data.schema_version != 1 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&data.json).ok()?;
    let scope = value.get("scope")?.as_str()?;
    if !matches!(scope, "local" | "global") {
        return None;
    }
    let mut template = String::new();
    for part in value.get("name")?.as_array()? {
        match part.get("kind")?.as_str()? {
            "text" => template.push_str(part.get("text")?.as_str()?),
            "expression" => {
                let index = usize::try_from(part.get("childIndex")?.as_u64()?).ok()?;
                if index >= node.children.len() {
                    return None;
                }
                template.push_str(&format!("<expression {}>", index + 1));
            }
            _ => return None,
        }
    }
    Some((scope.to_owned(), template))
}
