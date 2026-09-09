//! Serde DTOs that mirror every JSON object emitted by supported SSG schemas.
//!
//! These structures are a wire-format boundary, not the semantic parser model.
//! Optional values preserve nullable fields, while present empty lists remain
//! empty. Serde maps an omitted or JSON `null` optional field to `None`; the
//! normalized model may apply format-specific defaults during conversion.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub content_digest: String,
    pub generated_at: String,
    pub server: Server,
    pub language: String,
    pub plugins: Vec<Plugin>,
    pub capabilities: Capabilities,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub name: String,
    pub version: String,
    pub bukkit_version: String,
    pub minecraft_version: String,
    pub java_version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub load_order: usize,
    pub name: String,
    pub version: String,
    pub main: String,
    pub enabled: bool,
    pub depend: Vec<String>,
    pub soft_depend: Vec<String>,
    pub load_before: Vec<String>,
    pub jar_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub syntax_api: SyntaxApi,
    pub event_value_api: EventValueApi,
    pub syntax_kinds: SyntaxKindCapabilities,
    pub aliases: AliasCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyntaxApi {
    LegacyStatic,
    Registry,
}

impl SyntaxApi {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LegacyStatic => "legacy-static",
            Self::Registry => "registry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum EventValueApi {
    #[serde(rename = "legacy")]
    Legacy,
    #[serde(rename = "modern-2.15")]
    Modern215,
    #[serde(rename = "modern-2.16")]
    Modern216,
}

impl EventValueApi {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Modern215 => "modern-2.15",
            Self::Modern216 => "modern-2.16",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxKindCapabilities {
    pub conditions: bool,
    pub effects: bool,
    pub events: bool,
    pub expressions: bool,
    pub types: bool,
    pub functions: bool,
    pub sections: bool,
    pub structures: bool,
    pub properties: bool,
    pub arithmetic: bool,
    pub converters: bool,
    pub comparators: bool,
    pub event_values: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AliasCapabilities {
    pub supported: bool,
    pub collected: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Addon {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Priority {
    pub after: Vec<Priority>,
    pub before: Vec<Priority>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeMode {
    Add,
    Set,
    Remove,
    RemoveAll,
    Delete,
    Reset,
}

pub type ChangeModes = BTreeMap<ChangeMode, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolutionState {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReturnTypeState {
    Static,
    Dynamic,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PossibleReturnTypesState {
    Complete,
    Partial,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Multiplicity {
    Single,
    Multiple,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyntaxKind {
    Condition,
    Effect,
    Event,
    Expression,
    Section,
    Structure,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonSyntax {
    pub kind: SyntaxKind,
    pub registration_order: usize,
    pub name: Option<String>,
    pub id: Option<String>,
    pub documentation_id: Option<String>,
    pub element_class: String,
    pub super_class: Option<String>,
    pub since: Option<Vec<String>>,
    pub description: Option<Vec<String>>,
    pub examples: Option<Vec<String>>,
    pub keywords: Option<Vec<String>>,
    pub requires: Option<Vec<String>>,
    pub no_doc: bool,
    pub events: Option<Vec<String>>,
    pub deprecated: Option<bool>,
    pub priority_str: Option<String>,
    pub priority: Option<Priority>,
    pub patterns: Vec<String>,
    pub addon: Addon,
    pub definition_id: String,
    pub registration_id: String,
    pub related_property: Option<String>,
    pub supported_events: Option<Vec<String>>,
    pub supported_events_state: Option<ResolutionState>,
    pub experimental_syntax: Option<ExperimentalSyntax>,
    pub experimental_syntax_state: Option<ResolutionState>,
    pub return_handler: Option<ReturnHandler>,
    pub return_handler_state: Option<ResolutionState>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalSyntax {
    pub required: Vec<Experiment>,
    pub disallowed: Vec<Experiment>,
    pub error_message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Experiment {
    pub code_name: String,
    pub phase: String,
    pub known: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnHandler {
    pub return_value_type: Option<String>,
    pub single_return_value: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(flatten)]
    pub common: CommonSyntax,
    pub reference_events: Vec<String>,
    pub event_values: Vec<EventValue>,
    pub cancellable: bool,
    pub priority_supported: Option<bool>,
    pub has_on_prefix: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Expression {
    #[serde(flatten)]
    pub common: CommonSyntax,
    pub return_type: Option<String>,
    pub return_type_state: Option<ReturnTypeState>,
    pub possible_return_types: Option<Vec<String>>,
    pub possible_return_types_state: Option<PossibleReturnTypesState>,
    pub section_expression: bool,
    pub return_type_multiplicity: Option<Multiplicity>,
    pub return_type_multiplicity_state: ResolutionState,
    pub accepted_changers: Option<ChangeModes>,
    pub accepted_changers_state: ResolutionState,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    #[serde(flatten)]
    pub common: CommonSyntax,
    pub loop_section: bool,
    pub effect_section: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Structure {
    #[serde(flatten)]
    pub common: CommonSyntax,
    pub entry_validator: Option<EntryValidator>,
    pub node_type: Option<NodeType>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryValidator {
    pub entry_data: Vec<EntryData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryData {
    pub key: String,
    pub default_value: Option<Value>,
    pub optional: bool,
    pub multiple: bool,
    pub entry_data_class: String,
    pub kind: EntryKind,
    pub separator: Option<String>,
    pub value_type: Option<String>,
    pub string_mode: Option<String>,
    pub return_types: Option<Vec<String>>,
    pub flags: Option<i32>,
    pub nested_validator: Option<EntryValidator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Literal,
    VariableString,
    Expression,
    Trigger,
    Container,
    Section,
    KeyValue,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeType {
    Simple,
    Section,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ClassKind {
    Annotation,
    Enum,
    Interface,
    Array,
    Primitive,
    Record,
    Sealed,
    Synthetic,
    MemberClass,
    LocalClass,
    AnonymousClass,
    Class,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Type {
    pub type_parse_order: usize,
    pub name: Option<String>,
    pub description: Option<Vec<String>>,
    pub since: Option<Vec<String>>,
    pub examples: Option<Vec<String>>,
    pub keywords: Option<Vec<String>>,
    pub requires: Option<Vec<String>>,
    pub addon: Addon,
    pub definition_id: String,
    pub registration_id: String,
    pub documentation_id: Option<String>,
    pub has_docs: bool,
    pub changer: Option<ChangeModes>,
    pub original_class: String,
    pub class_type: ClassKind,
    pub code_name: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    pub assignable_to: Vec<String>,
    pub user_input_patterns: Option<Vec<String>>,
    pub noun: Noun,
    pub serialize_as: Option<String>,
    pub usage: Option<Vec<String>>,
    pub enum_values: Option<Vec<String>>,
    pub parser_patterns: Option<Vec<String>>,
    pub registered_parser_patterns: Option<Vec<RegisteredTypeParserPattern>>,
    pub literal_values: Option<Vec<String>>,
    pub type_literals: Option<Vec<TypeLiteral>>,
    pub parser_class: Option<String>,
    pub parse_contexts: Option<Vec<String>>,
    pub default_expression: Option<DefaultExpressionDescriptor>,
    /// Schema 3 through 5 representation, retained for compatibility.
    pub default_expression_class: Option<String>,
    pub has_parser: bool,
    pub has_serializer: bool,
    pub has_supplier: bool,
    pub properties: Vec<String>,
    pub before: Option<Vec<String>>,
    pub after: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultExpressionDescriptor {
    pub implementation_class: String,
    pub literal: bool,
    pub return_type: Option<String>,
    pub single: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredTypeParserPattern {
    pub pattern: String,
    pub registration_index: usize,
    pub pattern_index: usize,
    pub source_code_name: Option<String>,
    pub data_class: String,
    pub represented_class: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeLiteral {
    pub text: String,
    pub plural_text: Option<String>,
    pub variable_name: Option<String>,
    pub debug_text: Option<String>,
    pub value_class: String,
    pub represented_class: Option<String>,
    pub enum_constant: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Noun {
    pub key: String,
    pub value: Option<String>,
    pub singular: String,
    pub plural: String,
    pub gender: i32,
    pub gender_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Function {
    pub registration_order: usize,
    pub name: Option<String>,
    pub description: Option<Vec<String>>,
    pub since: Option<Vec<String>>,
    pub examples: Option<Vec<String>>,
    pub keywords: Option<Vec<String>>,
    pub requires: Option<Vec<String>>,
    pub return_type: Option<String>,
    pub return_type_is_single: bool,
    pub parameters: Vec<FunctionParameter>,
    pub addon: Addon,
    pub definition_id: String,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub parameter_type: String,
    pub modifiers: Vec<ParameterModifier>,
    pub single: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParameterModifier {
    #[serde(rename = "type")]
    pub kind: ParameterModifierKind,
    pub min: Option<Value>,
    pub max: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterModifierKind {
    Optional,
    Keyed,
    Range,
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Converter {
    pub from: String,
    pub to: String,
    pub flags: i32,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparator {
    pub registration_order: usize,
    pub first_type: String,
    pub second_type: String,
    pub supports_ordering: Option<bool>,
    pub supports_inversion: Option<bool>,
    pub addon: Addon,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventValue {
    pub event_class: String,
    pub value_class: String,
    pub time: i32,
    pub exclude_error_message: Option<String>,
    pub excludes: Option<Vec<String>>,
    pub resolution_order: usize,
    pub registration_order: Option<usize>,
    pub patterns: Option<Vec<String>>,
    pub accepted_changers: Option<ChangeModes>,
    pub context_dependent: Option<bool>,
    pub has_custom_input_validator: Option<bool>,
    pub has_custom_event_validator: Option<bool>,
    pub addon: Addon,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Property {
    pub name: String,
    pub documentation_id: String,
    pub description: String,
    pub since: Option<Vec<String>>,
    pub handler_class: String,
    pub related_types: Vec<TypeProperty>,
    pub addon: Addon,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeProperty {
    pub type_code_name: String,
    pub type_class: String,
    pub description: Option<String>,
    pub provider: Option<Addon>,
    pub handler_class: String,
    pub handler_kind: PropertyHandlerKind,
    pub return_type: Option<String>,
    pub possible_return_types: Option<Vec<String>>,
    pub accepted_changers: Option<ChangeModes>,
    pub requires_source_expression_change: Option<bool>,
    pub expression_metadata_state: Option<ResolutionState>,
    pub element_types: Option<Vec<String>>,
    pub supported_axes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyHandlerKind {
    Expression,
    Condition,
    Contains,
    TypedValue,
    Wxyz,
    Custom,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operator {
    pub sign: String,
    pub priority: Priority,
    pub key: Option<String>,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub operator_sign: String,
    pub left: String,
    pub right: String,
    pub return_type: String,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: String,
}

pub type Operations = BTreeMap<String, Vec<Operation>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    #[serde(rename = "type")]
    pub input_type: String,
    pub return_type: String,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Class {
    pub name: String,
    pub binary_name: String,
    pub kind: ClassKind,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    pub component_type: Option<String>,
    pub container_element_type: Option<String>,
    pub methods: Option<Vec<ClassMethod>>,
    pub provider: Option<Addon>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassMethod {
    pub name: String,
    pub parameter_types: Vec<String>,
    pub return_type: String,
    #[serde(rename = "static")]
    pub is_static: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Aliases {
    pub aliases: BTreeMap<String, usize>,
    pub targets: Vec<AliasTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AliasTarget {
    pub amount: i32,
    pub all: bool,
    pub types: Vec<AliasItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasItem {
    pub material: String,
    pub minecraft_id: Option<String>,
    pub durability: i32,
    pub plain: bool,
    pub alias: bool,
    pub block_values: Option<Value>,
    pub item_meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluralRules {
    pub algorithm: PluralAlgorithm,
    pub plural_override_supported: bool,
    pub rules: Vec<PluralRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluralAlgorithm {
    LegacyFirstMatch,
    SingularAware,
    Unresolved,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluralRule {
    pub rule_order: usize,
    pub singular: String,
    pub plural: String,
    pub complete_word: Option<bool>,
    pub origin: PluralRuleOrigin,
    pub override_registration_order: Option<usize>,
    pub addon: Addon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluralRuleOrigin {
    BuiltIn,
    Override,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub conditions: Vec<CommonSyntax>,
    pub effects: Vec<CommonSyntax>,
    pub events: Vec<Event>,
    pub expressions: Vec<Expression>,
    pub sections: Vec<Section>,
    pub structures: Vec<Structure>,
    pub types: Vec<Type>,
    pub functions: Vec<Function>,
    pub converters: Vec<Converter>,
    pub comparators: Vec<Comparator>,
    pub event_values: Vec<EventValue>,
    pub properties: Vec<Property>,
    pub operators: Vec<Operator>,
    pub operations: Operations,
    pub differences: Vec<Difference>,
    pub classes: Vec<Class>,
    pub aliases: Aliases,
    pub plural_rules: PluralRules,
    /// Effective global language key/value entries for schema 5 snapshots.
    pub language: BTreeMap<String, String>,
}
