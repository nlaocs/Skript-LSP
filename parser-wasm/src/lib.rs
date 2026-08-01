use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "host")]
pub mod bindings;
#[cfg(feature = "host")]
pub mod host;
#[cfg(feature = "host")]
pub mod state;

#[cfg(feature = "host")]
pub use host::{
    HostConfig, HostError, ParserHost, TreeMacroCall, TreeMacroRequest, TreeMacroResult,
    WasmPatternMatchResult,
};
#[cfg(feature = "host")]
pub use state::{ParseTransaction, StateError, StateStore};

pub const ABI_VERSION: AbiVersion = AbiVersion::new(1, 3);

pub const CAPABILITY_HOOKS: &str = "parser.hooks";
pub const CAPABILITY_STATE_STORE: &str = "parser.state-store";
pub const CAPABILITY_DYNAMIC_SYNTAX: &str = "parser.dynamic-syntax";
pub const CAPABILITY_TEXT_MACRO: &str = "parser.macro.text";
pub const CAPABILITY_TREE_MACRO: &str = "parser.macro.tree";
pub const CAPABILITY_AST_MACRO: &str = "parser.macro.ast";
pub const CAPABILITY_CONTEXT_UPDATES: &str = "parser.context-updates";
pub const CAPABILITY_ADDITIONAL_PARSE: &str = "parser.additional-parse";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbiVersion {
    pub major: u16,
    pub minor: u16,
}

impl AbiVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl std::fmt::Display for AbiVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    pub id: String,
    pub version: u32,
}

impl Capability {
    pub fn new(id: impl Into<String>, version: u32) -> Self {
        Self {
            id: id.into(),
            version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequirement {
    pub id: String,
    pub minimum_version: u32,
    pub required: bool,
}

impl CapabilityRequirement {
    pub fn required(id: impl Into<String>, minimum_version: u32) -> Self {
        Self {
            id: id.into(),
            minimum_version,
            required: true,
        }
    }

    pub fn optional(id: impl Into<String>, minimum_version: u32) -> Self {
        Self {
            id: id.into(),
            minimum_version,
            required: false,
        }
    }
}

/// Validates one side of the ABI negotiation.
///
/// The same function is used by the host for an addon manifest and by a guest
/// for the host profile passed to its initialization hook.
pub fn validate_compatibility(
    expected_abi: AbiVersion,
    actual_abi: AbiVersion,
    requirements: &[CapabilityRequirement],
    available: &[Capability],
) -> Result<(), CompatibilityError> {
    if expected_abi != actual_abi {
        return Err(CompatibilityError::AbiVersionMismatch {
            expected: expected_abi,
            actual: actual_abi,
        });
    }

    let mut available_by_id = BTreeMap::new();
    for capability in available {
        validate_capability_id(&capability.id)?;
        if available_by_id
            .insert(capability.id.as_str(), capability.version)
            .is_some()
        {
            return Err(CompatibilityError::DuplicateCapability {
                id: capability.id.clone(),
            });
        }
    }

    let mut requirement_ids = BTreeSet::new();
    for requirement in requirements {
        validate_capability_id(&requirement.id)?;
        if !requirement_ids.insert(requirement.id.as_str()) {
            return Err(CompatibilityError::DuplicateCapability {
                id: requirement.id.clone(),
            });
        }

        match available_by_id.get(requirement.id.as_str()).copied() {
            Some(actual) if actual < requirement.minimum_version && requirement.required => {
                return Err(CompatibilityError::CapabilityVersionTooOld {
                    id: requirement.id.clone(),
                    minimum: requirement.minimum_version,
                    actual,
                });
            }
            Some(_) => {}
            None if requirement.required => {
                return Err(CompatibilityError::MissingRequiredCapability {
                    id: requirement.id.clone(),
                    minimum: requirement.minimum_version,
                });
            }
            None => {}
        }
    }
    Ok(())
}

fn validate_capability_id(id: &str) -> Result<(), CompatibilityError> {
    if id.trim().is_empty() {
        Err(CompatibilityError::BlankCapabilityId)
    } else {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CompatibilityError {
    #[error("ABI version mismatch: expected {expected}, found {actual}")]
    AbiVersionMismatch {
        expected: AbiVersion,
        actual: AbiVersion,
    },
    #[error("capability ID must not be blank")]
    BlankCapabilityId,
    #[error("capability {id} is declared more than once")]
    DuplicateCapability { id: String },
    #[error("required capability {id} version {minimum} is unavailable")]
    MissingRequiredCapability { id: String, minimum: u32 },
    #[error(
        "capability {id} requires at least version {minimum}, but version {actual} is available"
    )]
    CapabilityVersionTooOld {
        id: String,
        minimum: u32,
        actual: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_abi_version_mismatches_explicitly() {
        let error =
            validate_compatibility(ABI_VERSION, AbiVersion::new(2, 0), &[], &[]).unwrap_err();
        assert_eq!(
            error,
            CompatibilityError::AbiVersionMismatch {
                expected: ABI_VERSION,
                actual: AbiVersion::new(2, 0),
            }
        );
    }

    #[test]
    fn rejects_unknown_required_capabilities_but_allows_optional_ones() {
        let required = CapabilityRequirement::required("future.required", 1);
        assert!(matches!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[required], &[]),
            Err(CompatibilityError::MissingRequiredCapability { .. })
        ));

        let optional = CapabilityRequirement::optional("future.optional", 1);
        assert_eq!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[optional], &[]),
            Ok(())
        );

        let optional_newer = CapabilityRequirement::optional("future.optional", 2);
        let older = Capability::new("future.optional", 1);
        assert_eq!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[optional_newer], &[older]),
            Ok(())
        );
    }

    #[test]
    fn validates_capability_versions_and_duplicate_ids() {
        let requirements = [CapabilityRequirement::required(CAPABILITY_HOOKS, 2)];
        let available = [Capability::new(CAPABILITY_HOOKS, 1)];
        assert!(matches!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &requirements, &available),
            Err(CompatibilityError::CapabilityVersionTooOld { .. })
        ));

        let duplicate = [
            Capability::new(CAPABILITY_HOOKS, 1),
            Capability::new(CAPABILITY_HOOKS, 1),
        ];
        assert!(matches!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[], &duplicate),
            Err(CompatibilityError::DuplicateCapability { .. })
        ));
    }
}
