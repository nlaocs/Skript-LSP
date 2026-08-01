//! Java-compatible hashing for SSG content digests and snapshot identities.
//!
//! The byte order, file order, and manifest projection intentionally match the
//! generator; changing them is a schema compatibility change.

use crate::raw::{Capabilities, Manifest, Plugin, Server};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) fn content_digest(files: &BTreeMap<&'static str, String>) -> String {
    let mut digest = Sha256::new();
    for (index, (file_name, json)) in files.iter().enumerate() {
        if index > 0 {
            digest.update(b"|");
        }
        append_sized(&mut digest, file_name);
        append_sized(&mut digest, json);
    }
    to_hex(digest.finalize())
}

pub(crate) fn snapshot_id(manifest: &Manifest) -> String {
    let plugin_fingerprints = manifest
        .plugins
        .iter()
        .map(plugin_fingerprint)
        .collect::<Vec<_>>();
    let mut files = manifest.files.clone();
    files.sort();

    let encoded = fingerprint(&[
        manifest.schema_version.to_string(),
        manifest.content_digest.clone(),
        server_fingerprint(&manifest.server),
        manifest.language.clone(),
        fingerprint(&plugin_fingerprints),
        capabilities_fingerprint(&manifest.capabilities),
        fingerprint(&files),
    ]);
    sha256(encoded.as_bytes())
}

fn server_fingerprint(server: &Server) -> String {
    fingerprint(&[
        server.name.clone(),
        server.version.clone(),
        server.bukkit_version.clone(),
        server.minecraft_version.clone(),
        server.java_version.clone(),
    ])
}

fn plugin_fingerprint(plugin: &Plugin) -> String {
    fingerprint(&[
        plugin.load_order.to_string(),
        plugin.name.clone(),
        plugin.version.clone(),
        plugin.main.clone(),
        plugin.enabled.to_string(),
        plugin.depend.join(","),
        plugin.soft_depend.join(","),
        plugin.load_before.join(","),
        plugin.jar_sha256.clone().unwrap_or_default(),
    ])
}

fn capabilities_fingerprint(capabilities: &Capabilities) -> String {
    let kinds = &capabilities.syntax_kinds;
    let kind_bits = [
        kinds.conditions,
        kinds.effects,
        kinds.events,
        kinds.expressions,
        kinds.types,
        kinds.functions,
        kinds.sections,
        kinds.structures,
        kinds.properties,
        kinds.arithmetic,
        kinds.converters,
        kinds.comparators,
        kinds.event_values,
    ]
    .into_iter()
    .map(|value| if value { '1' } else { '0' })
    .collect::<String>();
    let aliases = format!(
        "{}:{}",
        u8::from(capabilities.aliases.supported),
        u8::from(capabilities.aliases.collected)
    );

    fingerprint(&[
        capabilities.syntax_api.as_str().to_owned(),
        capabilities.event_value_api.as_str().to_owned(),
        kind_bits,
        aliases,
    ])
}

fn fingerprint(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| format!("{}:{part}", java_len(part)))
        .collect::<Vec<_>>()
        .join("|")
}

fn append_sized(digest: &mut Sha256, value: &str) {
    digest.update(java_len(value).to_string().as_bytes());
    digest.update(b":");
    digest.update(value.as_bytes());
}

fn java_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn sha256(value: &[u8]) -> String {
    to_hex(Sha256::digest(value))
}

fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_length_counts_utf_16_code_units() {
        assert_eq!(java_len("ascii"), 5);
        assert_eq!(java_len("a\u{1f600}b"), 4);
    }
}
