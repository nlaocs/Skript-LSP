//! Runtime information supplied by the parser host during addon initialization.
//!
//! The component is intentionally stateless from the host's point of view, so
//! the current profile lives in a small process-local store. The same store is
//! used by native tests and by the WebAssembly component; no target-specific
//! synchronization is required.

use std::sync::{LazyLock, RwLock};

use crate::nlaocs::skript_parser_addon::types::{RegisteredHandlerBinding, RuntimeProfile};

static CURRENT_PROFILE: LazyLock<RwLock<Option<RuntimeProfile>>> =
    LazyLock::new(|| RwLock::new(None));
static REGISTERED_HANDLER_BINDINGS: LazyLock<RwLock<Vec<RegisteredHandlerBinding>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

const LATEST_SUPPORTED_SKRIPT: (u64, u64) = (2, 16);
const EARLIEST_SUPPORTED_SKRIPT: (u64, u64, u64) = (2, 6, 4);

/// Replaces the profile after the host has successfully validated it.
pub(crate) fn replace(
    profile: RuntimeProfile,
    registered_handler_bindings: Vec<RegisteredHandlerBinding>,
) {
    crate::language::clear();
    let mut current = CURRENT_PROFILE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *current = Some(profile);
    *REGISTERED_HANDLER_BINDINGS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = registered_handler_bindings;
}

/// Tests whether the host resolved a logical handler to the current SSG registration.
///
/// The host expands Definition, class-suffix, superclass, and other resolvable
/// targets to concrete registration IDs before constructing
/// [`RegisteredHandlerBinding`]. Opaque dynamic-handler IDs are matched by the
/// host without relying on Java class-name suffixes.
pub(crate) fn handler_matches(handler_id: &str, registration_id: &str) -> bool {
    let bindings = REGISTERED_HANDLER_BINDINGS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if bindings.iter().any(|binding| {
        binding.handler_id == handler_id
            && binding
                .registration_ids
                .iter()
                .any(|id| id == registration_id)
    }) {
        return true;
    }
    drop(bindings);

    #[cfg(target_arch = "wasm32")]
    {
        return crate::nlaocs::skript_parser_addon::catalog_data::registered_handler_matches(
            handler_id,
            registration_id,
        )
        .unwrap_or(false);
    }

    #[cfg(not(target_arch = "wasm32"))]
    false
}

/// Returns a snapshot of the profile most recently accepted by initialization.
pub(crate) fn current() -> Option<RuntimeProfile> {
    CURRENT_PROFILE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Compares the active Skript release by its numeric major/minor components.
pub(crate) fn skript_at_least(major: u64, minor: u64) -> Option<bool> {
    let profile = current()?;
    let version = profile.skript_version?;
    let version = parse_skript_version(&version)?;
    (version <= LATEST_SUPPORTED_SKRIPT).then_some(version >= (major, minor))
}

pub(crate) fn skript_at_least_patch(major: u64, minor: u64, patch: u64) -> Option<bool> {
    let profile = current()?;
    let version = profile.skript_version?;
    let version = parse_skript_patch_version(&version)?;
    (version.0 < LATEST_SUPPORTED_SKRIPT.0
        || (version.0 == LATEST_SUPPORTED_SKRIPT.0 && version.1 <= LATEST_SUPPORTED_SKRIPT.1))
        .then_some(version >= (major, minor, patch))
}

pub(crate) fn parse_skript_version(version: &str) -> Option<(u64, u64)> {
    parse_skript_patch_version(version).map(|(major, minor, _)| (major, minor))
}

pub(crate) fn parse_skript_patch_version(version: &str) -> Option<(u64, u64, u64)> {
    let numeric = version
        .trim()
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .next()?;
    let mut components = numeric.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    let patch = components
        .next()
        .map(str::parse)
        .transpose()
        .ok()?
        .unwrap_or(0);
    Some((major, minor, patch))
}

pub(crate) fn supports_skript_version(version: &str) -> bool {
    parse_skript_patch_version(version).is_some_and(|version| {
        version >= EARLIEST_SUPPORTED_SKRIPT && (version.0, version.1) <= LATEST_SUPPORTED_SKRIPT
    })
}

pub(crate) fn snapshot_schema_at_least(version: u32) -> Option<bool> {
    current()?
        .snapshot_schema_version
        .map(|schema| schema >= version)
}

#[cfg(test)]
mod tests {
    use super::{
        LATEST_SUPPORTED_SKRIPT, parse_skript_patch_version, parse_skript_version,
        supports_skript_version,
    };

    #[test]
    fn parses_supported_skript_version_shapes() {
        assert_eq!(parse_skript_version("2.15.4"), Some((2, 15)));
        assert_eq!(parse_skript_version("2.16.0-pre1"), Some((2, 16)));
        assert_eq!(parse_skript_version("unknown"), None);
        assert_eq!(parse_skript_version("2"), None);
        assert_eq!(parse_skript_patch_version("2.9.5"), Some((2, 9, 5)));
        assert_eq!(parse_skript_patch_version("2.9"), Some((2, 9, 0)));
        assert_eq!(LATEST_SUPPORTED_SKRIPT, (2, 16));
    }

    #[test]
    fn rejects_versions_outside_the_implemented_range() {
        assert!(supports_skript_version("2.6.4"));
        assert!(supports_skript_version("2.15.4"));
        assert!(supports_skript_version("2.16.0-pre1"));
        assert!(!supports_skript_version("2.6.3"));
        assert!(!supports_skript_version("2.17.0"));
        assert!(!supports_skript_version("unknown"));
    }
}
