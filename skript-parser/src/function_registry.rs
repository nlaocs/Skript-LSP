//! Owned, transactional Function declarations for document parsing.
//!
//! The registry deliberately separates two representations:
//!
//! - [`FunctionDeclaration`] keeps source-facing information such as the
//!   declaration span and default-value source.
//! - [`FunctionDefinition`] is the smaller immutable signature consumed by
//!   the existing Function call parser.
//!
//! A registry transaction never stores references into guest memory, parser
//! input, or callbacks.  A frozen [`FunctionRegistrySnapshot`] can therefore
//! safely be copied into an [`ExpressionParseEnvironment`] implementation and
//! used for more than one recursive parse.

use crate::{FunctionDefinition, FunctionParameterDefinition, TextRange};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use syntaxes::ClassName;
use thiserror::Error;

/// Parser ID used by definitions projected from document declarations.
pub const DOCUMENT_FUNCTION_PARSER_ID: &str = "document.function";

/// Scope of a Function declaration and the corresponding lookup behavior.
///
/// `Global` means that only global declarations are visible. `Local` means
/// that local declarations are searched first, followed by global
/// declarations whose signatures are not shadowed by a local declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionScope {
    /// A project/document-wide Function.
    Global,
    /// A Function local to the current document scope.
    Local,
}

/// Alias used by adapters that want to make the lookup operation explicit.
pub type FunctionLookupScope = FunctionScope;

/// Version-dependent Function declaration and lookup capabilities.
///
/// The two named presets capture the versions currently used by the parser:
/// Skript 2.15.4 supports local Functions and overloads, while the 2.6.4
/// loader exposes only global, non-overloaded Functions.  The other flags
/// remain independent so an adapter can describe an intermediate or addon
/// specific dialect without changing the registry implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionVersionPolicy {
    /// Whether declarations with [`FunctionScope::Local`] are accepted.
    pub allow_local_functions: bool,
    /// Whether more than one signature with the same name is accepted.
    pub allow_overloads: bool,
    /// Whether parameters may carry a default source string.
    pub allow_default_values: bool,
    /// Whether the projected call definition accepts `name: value` syntax.
    pub allow_named_arguments: bool,
    /// Whether a parameter may receive a list rather than one value.
    pub allow_plural_parameters: bool,
    /// Whether a Function name may begin with `_` (Skript 2.12+).
    pub allow_leading_underscore: bool,
    /// Whether `returns` may introduce the return type (Skript 2.8+).
    pub allow_returns_keyword: bool,
    /// Whether `->` may introduce the return type (Skript 2.14+).
    pub allow_arrow_return: bool,
    /// Whether named call arguments use the broad 2.15+ name grammar.
    pub wide_named_argument_names: bool,
    /// Whether parameter names are compared case-insensitively.
    pub case_insensitive_parameters: bool,
}

impl FunctionVersionPolicy {
    /// Policy for the modern Function model used by Skript 2.15.4.
    pub const SKRIPT_2_15_4: Self = Self {
        allow_local_functions: true,
        allow_overloads: true,
        allow_default_values: true,
        allow_named_arguments: true,
        allow_plural_parameters: true,
        allow_leading_underscore: true,
        allow_returns_keyword: true,
        allow_arrow_return: true,
        wide_named_argument_names: true,
        case_insensitive_parameters: true,
    };

    /// Policy for the legacy Function loader used by Skript 2.6.4.
    pub const SKRIPT_2_6_4: Self = Self {
        allow_local_functions: false,
        allow_overloads: false,
        allow_default_values: true,
        allow_named_arguments: false,
        allow_plural_parameters: true,
        allow_leading_underscore: false,
        allow_returns_keyword: false,
        allow_arrow_return: false,
        wide_named_argument_names: false,
        case_insensitive_parameters: true,
    };

    /// Returns the modern preset.
    pub const fn modern() -> Self {
        Self::SKRIPT_2_15_4
    }

    /// Returns the 2.6.4-compatible preset.
    pub const fn legacy_2_6_4() -> Self {
        Self::SKRIPT_2_6_4
    }

    /// Selects the exact stable-release feature boundaries using Skript's
    /// default case-insensitive variable-name setting.
    pub const fn for_skript_version(major: u32, minor: u32, patch: u32) -> Self {
        Self::for_skript_version_with_case_insensitive_variables(major, minor, patch, true)
    }

    /// Selects version capabilities and the effective variable-name setting.
    ///
    /// Before 2.8 parameter names were always normalized to lowercase.
    /// From 2.8 onward duplicate checks follow `case-insensitive variables`.
    pub const fn for_skript_version_with_case_insensitive_variables(
        major: u32,
        minor: u32,
        _patch: u32,
        case_insensitive_variables: bool,
    ) -> Self {
        Self {
            allow_local_functions: is_at_least_skript_minor(major, minor, 7),
            allow_overloads: is_at_least_skript_minor(major, minor, 12),
            allow_default_values: true,
            allow_named_arguments: is_at_least_skript_minor(major, minor, 14),
            allow_plural_parameters: true,
            allow_leading_underscore: is_at_least_skript_minor(major, minor, 12),
            allow_returns_keyword: is_at_least_skript_minor(major, minor, 8),
            allow_arrow_return: is_at_least_skript_minor(major, minor, 14),
            wide_named_argument_names: is_at_least_skript_minor(major, minor, 15),
            case_insensitive_parameters: if is_at_least_skript_minor(major, minor, 8) {
                case_insensitive_variables
            } else {
                true
            },
        }
    }
}

const fn is_at_least_skript_minor(major: u32, minor: u32, required_minor: u32) -> bool {
    major > 2 || major == 2 && minor >= required_minor
}

impl Default for FunctionVersionPolicy {
    fn default() -> Self {
        Self::SKRIPT_2_15_4
    }
}

/// Return contract kept by a Function declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionReturnContract {
    /// The Java class returned by the Function, or `None` for no return value.
    pub return_type: Option<ClassName>,
    /// Whether the return value is one value instead of a list.
    pub single: bool,
}

impl FunctionReturnContract {
    /// Creates a no-return contract.
    pub const fn none() -> Self {
        Self {
            return_type: None,
            single: true,
        }
    }

    /// Creates a single-value return contract.
    pub fn single(return_type: ClassName) -> Self {
        Self {
            return_type: Some(return_type),
            single: true,
        }
    }

    /// Creates a plural/list return contract.
    pub fn multiple(return_type: ClassName) -> Self {
        Self {
            return_type: Some(return_type),
            single: false,
        }
    }
}

impl Default for FunctionReturnContract {
    fn default() -> Self {
        Self::none()
    }
}

/// Source-facing parameter information from a Function declaration.
///
/// Plurality is represented by `single == false`, matching Skript's
/// `numbers`/`number` distinction without inventing a `number[]` type.  A
/// default expression remains source text in this registry. The CoreLibrary
/// Structure handler validates it through the host Expression parser before
/// registering the declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionParameterDeclaration {
    /// Parameter name used by named arguments and local bindings.
    pub name: String,
    /// Declared Java/Skript value class.
    pub parameter_type: ClassName,
    /// Whether the parameter accepts exactly one value.
    pub single: bool,
    /// Source of the default expression, if the declaration has one.
    pub default_source: Option<String>,
}

impl FunctionParameterDeclaration {
    /// Creates a required single or plural parameter.
    pub fn required(name: impl Into<String>, parameter_type: ClassName, single: bool) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            single,
            default_source: None,
        }
    }

    /// Creates an optional parameter with an owned default source string.
    pub fn defaulted(
        name: impl Into<String>,
        parameter_type: ClassName,
        single: bool,
        default_source: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            single,
            default_source: Some(default_source.into()),
        }
    }

    /// Returns whether the parameter has a default expression.
    pub fn is_defaulted(&self) -> bool {
        self.default_source.is_some()
    }
}

/// Complete source-facing Function declaration.
///
/// `source` and every nested string are owned copies.  `span` is relative to
/// `source`, so a declaration can be retained after the original parser input
/// or WASM memory has been released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDeclaration {
    /// Original declaration text.
    pub source: String,
    /// Declaration range within `source`.
    pub span: TextRange,
    /// Visibility of the declaration.
    pub scope: FunctionScope,
    /// Function name used by call lookup.
    pub name: String,
    /// Ordered source-facing parameter declarations.
    pub parameters: Vec<FunctionParameterDeclaration>,
    /// Return type and multiplicity contract.
    pub return_contract: FunctionReturnContract,
    /// Extensible metadata preserved across projection.
    pub metadata: BTreeMap<String, String>,
}

impl FunctionDeclaration {
    /// Creates an owned declaration with empty extension metadata.
    pub fn new(
        source: impl Into<String>,
        span: TextRange,
        scope: FunctionScope,
        name: impl Into<String>,
        parameters: Vec<FunctionParameterDeclaration>,
        return_contract: FunctionReturnContract,
    ) -> Self {
        Self {
            source: source.into(),
            span,
            scope,
            name: name.into(),
            parameters,
            return_contract,
            metadata: BTreeMap::new(),
        }
    }

    /// Adds one owned metadata value and returns the declaration for chaining.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Validates source, parameters, and policy-specific capabilities before
    /// a declaration mutates a transaction.
    pub fn validate(&self, policy: FunctionVersionPolicy) -> Result<(), FunctionRegistryError> {
        if !is_valid_function_name(&self.name, policy.allow_leading_underscore) {
            return Err(FunctionRegistryError::InvalidFunctionName);
        }
        if !self.span.is_valid_for(&self.source) {
            return Err(FunctionRegistryError::InvalidDeclarationSpan { span: self.span });
        }
        if self.scope == FunctionScope::Local && !policy.allow_local_functions {
            return Err(FunctionRegistryError::LocalFunctionsUnsupported {
                name: self.name.clone(),
            });
        }
        match self
            .metadata
            .get("function.return-syntax")
            .map(String::as_str)
        {
            Some("returns") if !policy.allow_returns_keyword => {
                return Err(FunctionRegistryError::ReturnsKeywordUnsupported {
                    name: self.name.clone(),
                });
            }
            Some("arrow") if !policy.allow_arrow_return => {
                return Err(FunctionRegistryError::ArrowReturnUnsupported {
                    name: self.name.clone(),
                });
            }
            _ => {}
        }

        let mut names = HashSet::new();
        for parameter in &self.parameters {
            if parameter.name.trim().is_empty() {
                return Err(FunctionRegistryError::InvalidParameterName {
                    function: self.name.clone(),
                });
            }
            let comparison_name = if policy.case_insensitive_parameters {
                parameter.name.to_lowercase()
            } else {
                parameter.name.clone()
            };
            if !names.insert(comparison_name) {
                return Err(FunctionRegistryError::DuplicateParameter {
                    function: self.name.clone(),
                    parameter: parameter.name.clone(),
                });
            }
            if !parameter.single && !policy.allow_plural_parameters {
                return Err(FunctionRegistryError::PluralParametersUnsupported {
                    function: self.name.clone(),
                    parameter: parameter.name.clone(),
                });
            }
            if parameter.default_source.is_some() && !policy.allow_default_values {
                return Err(FunctionRegistryError::DefaultValuesUnsupported {
                    function: self.name.clone(),
                    parameter: parameter.name.clone(),
                });
            }
        }
        Ok(())
    }
}

/// One accepted declaration together with its immutable call projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRegistration {
    /// Complete source-facing declaration.
    pub declaration: FunctionDeclaration,
    /// Call-parser projection derived from `declaration`.
    pub definition: FunctionDefinition,
}

/// An accepted global declaration supplied by a current project document.
/// The caller owns project generation checks; native lookup preserves this
/// document/revision witness and never mutates the supplying registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFunction {
    pub definition: FunctionDefinition,
    pub document_id: String,
    pub document_revision: u64,
}

/// Dependency observations deliberately survive speculative candidate rollback.
/// An empty lookup can become successful after another document changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionLookupObservation {
    pub name: String,
    pub scope: FunctionScope,
    pub definitions: Vec<FunctionDefinition>,
}

/// A savepoint into one Function registry transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionRegistrySavepoint {
    transaction_id: u64,
    registration_count: usize,
    next_registration_order: usize,
}

impl FunctionRegistrySavepoint {
    /// Number of registrations retained by this savepoint.
    pub const fn registration_count(self) -> usize {
        self.registration_count
    }
}

/// Errors raised while registering or freezing Function declarations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FunctionRegistryError {
    /// An import is not an accepted global definition with matching provenance.
    #[error("invalid imported function: {reason}")]
    InvalidImport { reason: String },
    /// The transaction no longer accepts mutations.
    #[error("function registry transaction is frozen")]
    Frozen,
    /// The savepoint was created by another transaction.
    #[error("function registry savepoint belongs to another transaction")]
    InvalidSavepoint,
    /// The declaration has no usable name.
    #[error("function declaration has an empty name")]
    InvalidFunctionName,
    /// The declaration span is not a valid UTF-8 range in its source.
    #[error("function declaration span {span} is invalid for its source")]
    InvalidDeclarationSpan { span: TextRange },
    /// A parameter name is empty.
    #[error("function {function:?} has an empty parameter name")]
    InvalidParameterName { function: String },
    /// Two parameters have the same named-argument/local-binding name.
    #[error("function {function:?} declares parameter {parameter:?} more than once")]
    DuplicateParameter { function: String, parameter: String },
    /// A local declaration is not available in the selected Skript policy.
    #[error("function {name:?} cannot be local in this Skript version")]
    LocalFunctionsUnsupported { name: String },
    /// A plural/list parameter is not available in the selected policy.
    #[error(
        "function {function:?} parameter {parameter:?} cannot be plural in this Skript version"
    )]
    PluralParametersUnsupported { function: String, parameter: String },
    /// Default expressions are not available in the selected policy.
    #[error(
        "function {function:?} parameter {parameter:?} cannot have a default in this Skript version"
    )]
    DefaultValuesUnsupported { function: String, parameter: String },
    /// The `returns` spelling is not available before Skript 2.8.
    #[error("function {name:?} cannot use `returns` in this Skript version")]
    ReturnsKeywordUnsupported { name: String },
    /// The `->` spelling is not available before Skript 2.14.
    #[error("function {name:?} cannot use `->` in this Skript version")]
    ArrowReturnUnsupported { name: String },
    /// A second declaration with the same name and scope is not allowed.
    #[error("function {name:?} cannot be overloaded in this Skript version")]
    OverloadsUnsupported { name: String },
    /// An identical signature was registered twice in one scope.
    #[error("duplicate {scope:?} signature for function {name:?}")]
    DuplicateSignature { name: String, scope: FunctionScope },
}

/// Mutable, document/revision-bound Function registry.
///
/// Savepoints make speculative registration cheap to roll back. Rejected
/// Structure candidates may also remove their provisional declaration by span.
pub struct FunctionRegistryTransaction {
    document_id: String,
    revision: u64,
    policy: FunctionVersionPolicy,
    transaction_id: u64,
    registrations: Vec<FunctionRegistration>,
    imported: Vec<ImportedFunction>,
    lookups: RefCell<Vec<FunctionLookupObservation>>,
    next_registration_order: usize,
    frozen: bool,
}

static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

impl std::fmt::Debug for FunctionRegistryTransaction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FunctionRegistryTransaction")
            .field("document_id", &self.document_id)
            .field("revision", &self.revision)
            .field("policy", &self.policy)
            .field("registrations", &self.registrations)
            .field("next_registration_order", &self.next_registration_order)
            .field("frozen", &self.frozen)
            .finish()
    }
}

impl FunctionRegistryTransaction {
    /// Starts an owned registry transaction for one document revision.
    pub fn new(
        document_id: impl Into<String>,
        revision: u64,
        policy: FunctionVersionPolicy,
    ) -> Self {
        Self {
            document_id: document_id.into(),
            revision,
            policy,
            transaction_id: NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed),
            registrations: Vec::new(),
            imported: Vec::new(),
            lookups: RefCell::new(Vec::new()),
            next_registration_order: 0,
            frozen: false,
        }
    }

    /// Starts a transaction using the modern default policy.
    pub fn with_default_policy(document_id: impl Into<String>, revision: u64) -> Self {
        Self::new(document_id, revision, FunctionVersionPolicy::default())
    }

    /// Returns the canonical document identity bound to this transaction.
    pub fn document_id(&self) -> &str {
        &self.document_id
    }

    /// Returns the document revision bound to this transaction.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the version policy used for registration validation.
    pub const fn policy(&self) -> FunctionVersionPolicy {
        self.policy
    }

    /// Returns whether this transaction has been frozen.
    pub const fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Installs a complete, immutable project input before declarations or
    /// lookups begin. This is not transactional guest state.
    pub fn import_functions(
        &mut self,
        imported: Vec<ImportedFunction>,
    ) -> Result<(), FunctionRegistryError> {
        self.ensure_mutable()?;
        if !self.registrations.is_empty()
            || !self.lookups.borrow().is_empty()
            || !self.imported.is_empty()
        {
            return Err(FunctionRegistryError::InvalidImport {
                reason: "imports must be installed once before declarations and lookups".into(),
            });
        }
        let mut identities = HashSet::new();
        let mut signatures = HashSet::new();
        let mut names = HashSet::new();
        for item in &imported {
            validate_import(item, &self.document_id, self.policy)?;
            let definition = &item.definition;
            if !identities.insert((
                definition.definition_id.clone(),
                definition.registration_id.clone(),
            )) {
                return Err(FunctionRegistryError::InvalidImport {
                    reason: "duplicate imported definition identity".into(),
                });
            }
            if !signatures.insert((definition.name.clone(), definition_shape(definition))) {
                return Err(FunctionRegistryError::DuplicateSignature {
                    name: definition.name.clone(),
                    scope: FunctionScope::Global,
                });
            }
            if !self.policy.allow_overloads && !names.insert(definition.name.clone()) {
                return Err(FunctionRegistryError::OverloadsUnsupported {
                    name: definition.name.clone(),
                });
            }
        }
        self.imported = imported;
        Ok(())
    }

    /// Complete callable input for sound native-unit cache equality. Imported
    /// owner/revision witnesses remain available separately in `imports()`.
    pub fn callable_definitions(&self) -> Vec<FunctionDefinition> {
        self.registrations
            .iter()
            .map(|registration| registration.definition.clone())
            .chain(self.imported.iter().map(|item| item.definition.clone()))
            .collect()
    }
    pub fn imports(&self) -> &[ImportedFunction] {
        &self.imported
    }
    pub fn lookup_observations(&self) -> Vec<FunctionLookupObservation> {
        self.lookups.borrow().clone()
    }

    /// Registers one declaration and returns its owned projection.
    pub fn register(
        &mut self,
        declaration: FunctionDeclaration,
    ) -> Result<FunctionRegistration, FunctionRegistryError> {
        self.ensure_mutable()?;
        declaration.validate(self.policy)?;

        // Namespace absence and conflicting imported overloads are reads too.
        // Keep them even if this provisional header is later rejected.
        if declaration.scope == FunctionScope::Global {
            self.lookup_functions(&declaration.name, FunctionScope::Global);
        }

        let shape = signature_shape(&declaration);
        let same_scope = self
            .registrations
            .iter()
            .filter(|registration| {
                registration.declaration.name == declaration.name
                    && registration.declaration.scope == declaration.scope
            })
            .collect::<Vec<_>>();
        let duplicate = same_scope
            .iter()
            .any(|registration| signature_shape(&registration.declaration) == shape);
        let imported_name = declaration.scope == FunctionScope::Global
            && self
                .imported
                .iter()
                .any(|item| item.definition.name == declaration.name);
        let imported_duplicate = declaration.scope == FunctionScope::Global
            && self.imported.iter().any(|item| {
                item.definition.name == declaration.name
                    && definition_shape(&item.definition) == shape
            });
        if duplicate || imported_duplicate {
            return Err(FunctionRegistryError::DuplicateSignature {
                name: declaration.name.clone(),
                scope: declaration.scope,
            });
        }
        if !self.policy.allow_overloads && (!same_scope.is_empty() || imported_name) {
            return Err(FunctionRegistryError::OverloadsUnsupported {
                name: declaration.name.clone(),
            });
        }

        let registration_order = self.next_registration_order;
        self.next_registration_order = self.next_registration_order.saturating_add(1);
        let definition_id = format!(
            "function:document:{}:{}",
            self.document_id, registration_order
        );
        let registration_id = format!("{definition_id}:0");
        let definition = project_definition(
            &declaration,
            &self.document_id,
            self.revision,
            registration_order,
            definition_id,
            registration_id,
            self.policy,
        );
        let registration = FunctionRegistration {
            declaration,
            definition,
        };
        self.registrations.push(registration.clone());
        Ok(registration)
    }

    /// Removes provisional declarations emitted by one rejected Structure header.
    pub fn remove_declarations_in_span(
        &mut self,
        span: TextRange,
    ) -> Result<usize, FunctionRegistryError> {
        self.ensure_mutable()?;
        let before = self.registrations.len();
        self.registrations
            .retain(|registration| registration.declaration.span != span);
        Ok(before - self.registrations.len())
    }

    /// Looks up declarations already accepted in this live transaction.
    ///
    /// Structure parsing registers every Function header before parsing any
    /// body, so this view supports both forward references and recursion
    /// without freezing the document registry early.
    pub fn lookup_functions(
        &self,
        name: &str,
        scope: FunctionLookupScope,
    ) -> Vec<FunctionDefinition> {
        let definitions = lookup_with_imports(&self.registrations, &self.imported, name, scope);
        let observation = FunctionLookupObservation {
            name: name.into(),
            scope,
            definitions: definitions.clone(),
        };
        let mut observations = self.lookups.borrow_mut();
        if !observations.contains(&observation) {
            observations.push(observation);
        }
        definitions
    }

    /// Creates a rollback point for the current append-only state.
    pub fn savepoint(&self) -> FunctionRegistrySavepoint {
        FunctionRegistrySavepoint {
            transaction_id: self.transaction_id,
            registration_count: self.registrations.len(),
            next_registration_order: self.next_registration_order,
        }
    }

    /// Rolls the transaction back to a savepoint created by this transaction.
    pub fn rollback(
        &mut self,
        savepoint: FunctionRegistrySavepoint,
    ) -> Result<(), FunctionRegistryError> {
        self.ensure_mutable()?;
        if savepoint.transaction_id != self.transaction_id
            || savepoint.registration_count > self.registrations.len()
        {
            return Err(FunctionRegistryError::InvalidSavepoint);
        }
        self.registrations.truncate(savepoint.registration_count);
        self.next_registration_order = savepoint.next_registration_order;
        Ok(())
    }

    /// Freezes this revision into an immutable lookup snapshot.
    pub fn freeze(&mut self) -> Result<FunctionRegistrySnapshot, FunctionRegistryError> {
        self.ensure_mutable()?;
        self.frozen = true;
        Ok(FunctionRegistrySnapshot {
            document_id: self.document_id.clone(),
            revision: self.revision,
            policy: self.policy,
            registrations: self.registrations.clone(),
            imported: self.imported.clone(),
            lookups: self.lookups.borrow().clone(),
        })
    }

    fn ensure_mutable(&self) -> Result<(), FunctionRegistryError> {
        if self.frozen {
            Err(FunctionRegistryError::Frozen)
        } else {
            Ok(())
        }
    }
}

/// Immutable declarations and call definitions for one document revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRegistrySnapshot {
    document_id: String,
    revision: u64,
    policy: FunctionVersionPolicy,
    registrations: Vec<FunctionRegistration>,
    imported: Vec<ImportedFunction>,
    lookups: Vec<FunctionLookupObservation>,
}

impl FunctionRegistrySnapshot {
    /// Returns the document identity used to create this snapshot.
    pub fn document_id(&self) -> &str {
        &self.document_id
    }

    /// Returns the exact source revision represented by this snapshot.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the policy used to create this snapshot.
    pub const fn policy(&self) -> FunctionVersionPolicy {
        self.policy
    }

    /// Returns all accepted declaration/projection pairs in registration order.
    pub fn registrations(&self) -> &[FunctionRegistration] {
        &self.registrations
    }

    /// Returns all immutable call definitions in registration order.
    pub fn definitions(&self) -> Vec<FunctionDefinition> {
        self.registrations
            .iter()
            .map(|registration| registration.definition.clone())
            .chain(self.imported.iter().map(|item| item.definition.clone()))
            .collect()
    }

    /// Immutable supplying document identities and revisions.
    pub fn imports(&self) -> &[ImportedFunction] {
        &self.imported
    }
    /// Complete lookup dependencies, including empty and rejected attempts.
    pub fn lookups(&self) -> &[FunctionLookupObservation] {
        &self.lookups
    }

    /// Looks up a name using the selected global or local-first scope.
    ///
    /// For a local lookup, local declarations are returned first.  A global
    /// declaration is then returned only when no local declaration has the
    /// same parameter type and plurality signature.
    pub fn lookup(&self, name: &str, scope: FunctionScope) -> Vec<FunctionDefinition> {
        lookup_with_imports(&self.registrations, &self.imported, name, scope)
    }

    /// Name chosen to make direct use as an `ExpressionParseEnvironment`
    /// adapter self-documenting.
    pub fn lookup_functions(
        &self,
        name: &str,
        scope: FunctionLookupScope,
    ) -> Vec<FunctionDefinition> {
        self.lookup(name, scope)
    }

    /// Returns matching registrations without cloning their definitions.
    pub fn lookup_references(
        &self,
        name: &str,
        scope: FunctionScope,
    ) -> Vec<&FunctionRegistration> {
        lookup_references(&self.registrations, name, scope)
    }

    /// Convenience lookup that makes the local-before-global rule explicit.
    pub fn lookup_local_then_global(&self, name: &str) -> Vec<FunctionDefinition> {
        self.lookup(name, FunctionScope::Local)
    }

    /// Returns the number of accepted declarations in this revision.
    pub const fn len(&self) -> usize {
        self.registrations.len()
    }

    /// Returns whether this snapshot contains no Function declarations.
    pub const fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }
}

fn lookup_references<'a>(
    registrations: &'a [FunctionRegistration],
    name: &str,
    scope: FunctionScope,
) -> Vec<&'a FunctionRegistration> {
    let mut local = Vec::new();
    let mut global = Vec::new();
    for registration in registrations {
        if registration.declaration.name != name {
            continue;
        }
        match registration.declaration.scope {
            FunctionScope::Local => local.push(registration),
            FunctionScope::Global => global.push(registration),
        }
    }
    if scope == FunctionScope::Global {
        return global;
    }

    let local_shapes = local
        .iter()
        .map(|registration| signature_shape(&registration.declaration))
        .collect::<HashSet<_>>();
    local
        .into_iter()
        .chain(global.into_iter().filter(|registration| {
            !local_shapes.contains(&signature_shape(&registration.declaration))
        }))
        .collect()
}

fn signature_shape(declaration: &FunctionDeclaration) -> Vec<(ClassName, bool)> {
    declaration
        .parameters
        .iter()
        .map(|parameter| (parameter.parameter_type.clone(), parameter.single))
        .collect()
}

fn lookup_with_imports(
    registrations: &[FunctionRegistration],
    imported: &[ImportedFunction],
    name: &str,
    scope: FunctionScope,
) -> Vec<FunctionDefinition> {
    let local_shapes = registrations
        .iter()
        .filter(|registration| {
            registration.declaration.name == name
                && registration.declaration.scope == FunctionScope::Local
        })
        .map(|registration| signature_shape(&registration.declaration))
        .collect::<HashSet<_>>();
    lookup_references(registrations, name, scope)
        .into_iter()
        .map(|registration| registration.definition.clone())
        .chain(
            imported
                .iter()
                .filter(|item| {
                    item.definition.name == name
                        && (scope == FunctionScope::Global
                            || !local_shapes.contains(&definition_shape(&item.definition)))
                })
                .map(|item| item.definition.clone()),
        )
        .collect()
}

fn definition_shape(definition: &FunctionDefinition) -> Vec<(ClassName, bool)> {
    definition
        .parameters
        .iter()
        .map(|parameter| (parameter.parameter_type.clone(), parameter.single))
        .collect()
}

fn validate_import(
    item: &ImportedFunction,
    receiver: &str,
    policy: FunctionVersionPolicy,
) -> Result<(), FunctionRegistryError> {
    let definition = &item.definition;
    let invalid = |reason: &str| FunctionRegistryError::InvalidImport {
        reason: reason.into(),
    };
    if item.document_id.is_empty() || item.document_id == receiver {
        return Err(invalid("owner must identify another project document"));
    }
    if definition.parser_id != DOCUMENT_FUNCTION_PARSER_ID
        || definition.definition_id.is_empty()
        || definition.registration_id.is_empty()
    {
        return Err(invalid(
            "only accepted document function definitions can be imported",
        ));
    }
    let revision = item.document_revision.to_string();
    for (key, expected) in [
        ("function.scope", "global"),
        ("function.document-id", item.document_id.as_str()),
        ("function.revision", revision.as_str()),
    ] {
        if definition.metadata.get(key).map(String::as_str) != Some(expected) {
            return Err(invalid(
                "definition scope/owner/revision witness does not match import",
            ));
        }
    }
    if !is_valid_function_name(&definition.name, policy.allow_leading_underscore) {
        return Err(FunctionRegistryError::InvalidFunctionName);
    }
    let mut names = HashSet::new();
    for parameter in &definition.parameters {
        if parameter.name.trim().is_empty() || parameter.parameter_type.as_str().is_empty() {
            return Err(invalid("parameter name/type is empty"));
        }
        let comparison = if policy.case_insensitive_parameters {
            parameter.name.to_lowercase()
        } else {
            parameter.name.clone()
        };
        if !names.insert(comparison) {
            return Err(FunctionRegistryError::DuplicateParameter {
                function: definition.name.clone(),
                parameter: parameter.name.clone(),
            });
        }
        if !parameter.single && !policy.allow_plural_parameters {
            return Err(FunctionRegistryError::PluralParametersUnsupported {
                function: definition.name.clone(),
                parameter: parameter.name.clone(),
            });
        }
        if parameter.optional && !policy.allow_default_values {
            return Err(FunctionRegistryError::DefaultValuesUnsupported {
                function: definition.name.clone(),
                parameter: parameter.name.clone(),
            });
        }
    }
    if definition
        .return_type
        .as_ref()
        .is_some_and(|class| class.as_str().is_empty())
    {
        return Err(invalid("return type is empty"));
    }
    Ok(())
}

fn is_valid_function_name(name: &str, allow_leading_underscore: bool) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_alphabetic() || allow_leading_underscore && first == '_')
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn project_definition(
    declaration: &FunctionDeclaration,
    document_id: &str,
    revision: u64,
    registration_order: usize,
    definition_id: String,
    registration_id: String,
    policy: FunctionVersionPolicy,
) -> FunctionDefinition {
    let mut metadata = declaration.metadata.clone();
    metadata.insert(
        "function.scope".to_owned(),
        scope_name(declaration.scope).to_owned(),
    );
    metadata.insert("function.document-id".to_owned(), document_id.to_owned());
    metadata.insert("function.revision".to_owned(), revision.to_string());
    metadata.insert(
        "function.named-arguments".to_owned(),
        policy.allow_named_arguments.to_string(),
    );
    metadata.insert(
        "function.named-argument-pattern".to_owned(),
        if policy.wide_named_argument_names {
            "wide"
        } else {
            "ascii"
        }
        .to_owned(),
    );
    for (index, parameter) in declaration.parameters.iter().enumerate() {
        metadata.insert(
            format!("function.parameter.{index}.name"),
            parameter.name.clone(),
        );
        if let Some(default_source) = &parameter.default_source {
            metadata.insert(
                format!("function.parameter.{index}.default-source"),
                default_source.clone(),
            );
        }
    }

    FunctionDefinition {
        parser_id: DOCUMENT_FUNCTION_PARSER_ID.to_owned(),
        name: declaration.name.clone(),
        definition_id,
        registration_id,
        registration_order,
        return_type: declaration.return_contract.return_type.clone(),
        return_type_is_single: declaration.return_contract.single,
        parameters: declaration
            .parameters
            .iter()
            .map(|parameter| FunctionParameterDefinition {
                name: parameter.name.clone(),
                parameter_type: parameter.parameter_type.clone(),
                single: parameter.single,
                optional: parameter.default_source.is_some(),
            })
            .collect(),
        metadata,
    }
}

const fn scope_name(scope: FunctionScope) -> &'static str {
    match scope {
        FunctionScope::Global => "global",
        FunctionScope::Local => "local",
    }
}
