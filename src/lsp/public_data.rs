use parser_wasm::host::AstNode;

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
