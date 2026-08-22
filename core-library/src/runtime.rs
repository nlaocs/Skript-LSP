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

/// Replaces the profile after the host has successfully validated it.
pub(crate) fn replace(
    profile: RuntimeProfile,
    registered_handler_bindings: Vec<RegisteredHandlerBinding>,
) {
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
/// The host expands Definition and class-suffix targets to their concrete
/// registration IDs before constructing [`RegisteredHandlerBinding`].
pub(crate) fn handler_matches(handler_id: &str, registration_id: &str) -> bool {
    let bindings = REGISTERED_HANDLER_BINDINGS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    bindings.iter().any(|binding| {
        binding.handler_id == handler_id
            && binding
                .registration_ids
                .iter()
                .any(|id| id == registration_id)
    })
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
    let mut components = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse::<u64>().ok());
    Some((components.next()?, components.next()?) >= (major, minor))
}
