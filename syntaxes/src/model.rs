//! Format-independent domain model for Skript and addon registrations.
//!
//! The types in this module contain no file I/O and no SSG schema assumptions.
//! `Catalog` indexes them, while `ssg` is responsible for constructing them.
#![allow(missing_docs)] // Public fields are described by their owning domain type.

use serde_json::Value;
use std::collections::BTreeMap;
use syntax_pattern_parser::syntax::ParseResult;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

string_id!(ClassName);
string_id!(DefinitionId);
string_id!(RegistrationId);
string_id!(TypeCodeName);

#[derive(Debug, Clone, PartialEq, Eq)]
/// Plugin identity attached to a generated registration or registry entry.
pub struct Addon {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Whether runtime-derived metadata was resolved or unresolved.
pub enum ResolutionState {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Whether an expression's registered return type is exact or context-dependent.
pub enum ReturnTypeState {
    Static,
    Dynamic,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Completeness of the known runtime return-type alternatives.
pub enum PossibleReturnTypesState {
    Complete,
    Partial,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Whether an expression returns one value, many values, or supports both.
pub enum Multiplicity {
    Single,
    Multiple,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Mutation operation accepted by an expression changer.
pub enum ChangeMode {
    Add,
    Set,
    Remove,
    RemoveAll,
    Delete,
    Reset,
}

/// Map from changer operation to accepted Java value classes.
pub type ChangeModes = BTreeMap<ChangeMode, Vec<ClassName>>;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Ordering constraints declared around one syntax registration.
pub struct Priority {
    pub after: Vec<Priority>,
    pub before: Vec<Priority>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Optional documentation collected from a Skript registration.
pub struct Documentation {
    pub name: Option<String>,
    pub documentation_id: Option<String>,
    pub since: Vec<String>,
    pub description: Vec<String>,
    pub examples: Vec<String>,
    pub keywords: Vec<String>,
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parsed registration pattern and its original source text.
pub struct Pattern {
    pub source: String,
    pub parsed: ParseResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Experimental feature required to enable a syntax.
pub struct Experiment {
    pub code_name: String,
    pub phase: String,
    pub known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Experimental requirements and explicit disallowances for a syntax.
pub struct ExperimentalSyntax {
    pub required: Vec<Experiment>,
    pub disallowed: Vec<Experiment>,
    pub error_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Marker describing a syntax that supplies values to a surrounding section.
pub struct ReturnHandler {
    pub return_value_type: Option<ClassName>,
    pub single_return_value: bool,
}

#[derive(Debug, Clone, PartialEq)]
/// Normalized rules for entries accepted by a structure.
pub struct EntryValidator {
    pub entry_data: Vec<EntryData>,
}

#[derive(Debug, Clone, PartialEq)]
/// One named structure entry and its value requirements.
pub struct EntryData {
    pub key: String,
    pub default_value: Option<Value>,
    pub optional: bool,
    pub multiple: bool,
    pub entry_data_class: ClassName,
    pub kind: EntryKind,
    pub separator: Option<String>,
    pub value_type: Option<ClassName>,
    pub string_mode: Option<String>,
    pub return_types: Vec<ClassName>,
    pub flags: Option<i32>,
    pub nested_validator: Option<EntryValidator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Semantic kind of a structure entry validator.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Physical Skript node shape accepted for a structure entry.
pub enum NodeType {
    Simple,
    Section,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
/// Fields shared by every registered syntax category.
pub struct CommonSyntax {
    pub registration_order: usize,
    pub documentation: Documentation,
    pub id: Option<String>,
    pub element_class: ClassName,
    pub super_class: Option<ClassName>,
    pub no_doc: bool,
    pub events: Vec<String>,
    pub deprecated: Option<bool>,
    pub priority_name: Option<String>,
    pub priority: Option<Priority>,
    pub patterns: Vec<Pattern>,
    pub addon: Addon,
    pub definition_id: DefinitionId,
    pub registration_id: RegistrationId,
    pub related_property: Option<String>,
    pub supported_events: Option<Vec<ClassName>>,
    pub supported_events_state: Option<ResolutionState>,
    pub experimental_syntax: Option<ExperimentalSyntax>,
    pub experimental_syntax_state: Option<ResolutionState>,
    pub return_handler: Option<ReturnHandler>,
    pub return_handler_state: Option<ResolutionState>,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered event syntax and its event-value context.
pub struct Event {
    pub common: CommonSyntax,
    pub reference_events: Vec<ClassName>,
    pub event_values: Vec<EventValue>,
    pub cancellable: bool,
    /// Whether the SkriptEvent accepts an explicit Bukkit event priority.
    /// Older snapshots leave this unresolved.
    pub priority_supported: Option<bool>,
    pub has_on_prefix: bool,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered boolean condition syntax.
pub struct Condition {
    pub common: CommonSyntax,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered executable effect syntax.
pub struct Effect {
    pub common: CommonSyntax,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered value-producing expression syntax.
pub struct Expression {
    pub common: CommonSyntax,
    pub return_type: Option<ClassName>,
    pub return_type_state: ReturnTypeState,
    pub possible_return_types: Vec<ClassName>,
    pub possible_return_types_state: PossibleReturnTypesState,
    pub section_expression: bool,
    pub return_type_multiplicity: Option<Multiplicity>,
    pub return_type_multiplicity_state: ResolutionState,
    pub accepted_changers: Option<ChangeModes>,
    pub accepted_changers_state: ResolutionState,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered syntax that owns an indented body.
pub struct Section {
    pub common: CommonSyntax,
    pub loop_section: bool,
    pub effect_section: bool,
}

#[derive(Debug, Clone, PartialEq)]
/// Top-level structure syntax and entry-validation metadata.
pub struct Structure {
    pub common: CommonSyntax,
    pub entry_validator: Option<EntryValidator>,
    pub node_type: Option<NodeType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Role played by a Java class in the generated hierarchy.
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// Localized type display name with grammatical gender and plurality.
pub struct Noun {
    pub key: String,
    pub value: Option<String>,
    pub singular: String,
    pub plural: String,
    pub gender: i32,
    pub gender_id: String,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered Skript type, parser metadata, and Java representation.
pub struct Type {
    /// Zero-based position of this object in `Types.json`.
    pub source_index: usize,
    pub type_parse_order: usize,
    pub documentation: Documentation,
    pub addon: Addon,
    pub definition_id: DefinitionId,
    pub registration_id: RegistrationId,
    pub has_docs: bool,
    pub changer: Option<ChangeModes>,
    pub original_class: ClassName,
    pub class_type: ClassKind,
    pub code_name: TypeCodeName,
    pub super_class: Option<ClassName>,
    pub interfaces: Vec<ClassName>,
    pub assignable_to: Vec<TypeCodeName>,
    pub user_input_patterns: Vec<String>,
    pub noun: Noun,
    pub serialize_as: Option<ClassName>,
    pub usage: Vec<String>,
    pub enum_values: Vec<String>,
    pub parser_patterns: Vec<String>,
    pub registered_parser_patterns: Vec<RegisteredTypeParserPattern>,
    pub literal_values: Vec<String>,
    pub type_literals: Vec<TypeLiteral>,
    pub parser_class: Option<ClassName>,
    pub parse_contexts: Vec<String>,
    pub default_expression_class: Option<ClassName>,
    pub has_parser: bool,
    pub has_serializer: bool,
    pub has_supplier: bool,
    pub properties: Vec<String>,
    pub before: Vec<TypeCodeName>,
    pub after: Vec<TypeCodeName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One runtime syntax registration used by a Type parser.
pub struct RegisteredTypeParserPattern {
    pub pattern: String,
    pub registration_index: usize,
    pub pattern_index: usize,
    pub source_code_name: Option<String>,
    pub data_class: ClassName,
    pub represented_class: ClassName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One finite value exposed by a registered type's supplier and parser.
pub struct TypeLiteral {
    pub text: String,
    pub plural_text: Option<String>,
    pub variable_name: Option<String>,
    pub debug_text: Option<String>,
    pub value_class: ClassName,
    pub represented_class: Option<ClassName>,
    pub enum_constant: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
/// Cardinality modifier applied to a function parameter.
pub enum ParameterModifier {
    Optional,
    Keyed,
    Range { min: Value, max: Value },
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
/// One function parameter and its modifiers.
pub struct FunctionParameter {
    pub name: String,
    pub parameter_type: ClassName,
    pub modifiers: Vec<ParameterModifier>,
    pub single: bool,
}

#[derive(Debug, Clone, PartialEq)]
/// Registered Skript function signature and implementation metadata.
pub struct Function {
    pub registration_order: usize,
    pub name: String,
    pub documentation: Documentation,
    pub return_type: Option<ClassName>,
    pub return_type_is_single: bool,
    pub parameters: Vec<FunctionParameter>,
    pub addon: Addon,
    pub definition_id: DefinitionId,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Any of the eight syntax categories in canonical project order.
pub enum Syntax {
    Event(Event),
    Condition(Condition),
    Effect(Effect),
    Expression(Expression),
    Type(Type),
    Function(Function),
    Section(Section),
    Structure(Structure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Discriminant for the eight syntax categories.
pub enum SyntaxKind {
    Event,
    Condition,
    Effect,
    Expression,
    Type,
    Function,
    Section,
    Structure,
}

impl Syntax {
    pub fn kind(&self) -> SyntaxKind {
        match self {
            Self::Event(_) => SyntaxKind::Event,
            Self::Condition(_) => SyntaxKind::Condition,
            Self::Effect(_) => SyntaxKind::Effect,
            Self::Expression(_) => SyntaxKind::Expression,
            Self::Type(_) => SyntaxKind::Type,
            Self::Function(_) => SyntaxKind::Function,
            Self::Section(_) => SyntaxKind::Section,
            Self::Structure(_) => SyntaxKind::Structure,
        }
    }

    pub fn registration_id(&self) -> &RegistrationId {
        match self {
            Self::Event(value) => &value.common.registration_id,
            Self::Condition(value) => &value.common.registration_id,
            Self::Effect(value) => &value.common.registration_id,
            Self::Expression(value) => &value.common.registration_id,
            Self::Type(value) => &value.registration_id,
            Self::Function(value) => &value.registration_id,
            Self::Section(value) => &value.common.registration_id,
            Self::Structure(value) => &value.common.registration_id,
        }
    }

    pub fn definition_id(&self) -> &DefinitionId {
        match self {
            Self::Event(value) => &value.common.definition_id,
            Self::Condition(value) => &value.common.definition_id,
            Self::Effect(value) => &value.common.definition_id,
            Self::Expression(value) => &value.common.definition_id,
            Self::Type(value) => &value.definition_id,
            Self::Function(value) => &value.definition_id,
            Self::Section(value) => &value.common.definition_id,
            Self::Structure(value) => &value.common.definition_id,
        }
    }

    pub fn registration_order(&self) -> usize {
        match self {
            Self::Event(value) => value.common.registration_order,
            Self::Condition(value) => value.common.registration_order,
            Self::Effect(value) => value.common.registration_order,
            Self::Expression(value) => value.common.registration_order,
            Self::Type(value) => value.type_parse_order,
            Self::Function(value) => value.registration_order,
            Self::Section(value) => value.common.registration_order,
            Self::Structure(value) => value.common.registration_order,
        }
    }

    /// Returns metadata shared by pattern-based syntax kinds.
    pub fn common(&self) -> Option<&CommonSyntax> {
        match self {
            Self::Event(value) => Some(&value.common),
            Self::Condition(value) => Some(&value.common),
            Self::Effect(value) => Some(&value.common),
            Self::Expression(value) => Some(&value.common),
            Self::Type(_) | Self::Function(_) => None,
            Self::Section(value) => Some(&value.common),
            Self::Structure(value) => Some(&value.common),
        }
    }
}

impl SyntaxKind {
    pub const fn order(self) -> u8 {
        match self {
            Self::Event => 0,
            Self::Condition => 1,
            Self::Effect => 2,
            Self::Expression => 3,
            Self::Type => 4,
            Self::Function => 5,
            Self::Section => 6,
            Self::Structure => 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Directed conversion from one Java value class to another.
pub struct Converter {
    pub from: ClassName,
    pub to: ClassName,
    pub flags: i32,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Comparison handler registered for two Java value classes.
pub struct Comparator {
    pub registration_order: usize,
    pub first_type: ClassName,
    pub second_type: ClassName,
    pub supports_ordering: Option<bool>,
    pub supports_inversion: Option<bool>,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Value exposed while parsing a compatible event context.
pub struct EventValue {
    pub event_class: ClassName,
    pub value_class: ClassName,
    pub time: i32,
    pub exclude_error_message: Option<String>,
    pub excludes: Option<Vec<ClassName>>,
    pub resolution_order: usize,
    pub registration_order: Option<usize>,
    pub patterns: Option<Vec<String>>,
    pub accepted_changers: Option<ChangeModes>,
    pub context_dependent: Option<bool>,
    pub has_custom_input_validator: Option<bool>,
    pub has_custom_event_validator: Option<bool>,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Named property and the handlers registered for it.
pub struct Property {
    pub name: String,
    pub documentation_id: String,
    pub description: String,
    pub since: Vec<String>,
    pub handler_class: ClassName,
    pub related_types: Vec<TypeProperty>,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Property support declared by one Skript type.
pub struct TypeProperty {
    pub type_code_name: TypeCodeName,
    pub type_class: ClassName,
    pub description: Option<String>,
    pub provider: Option<Addon>,
    pub handler_class: ClassName,
    pub handler_kind: PropertyHandlerKind,
    pub return_type: Option<ClassName>,
    pub possible_return_types: Option<Vec<ClassName>>,
    pub accepted_changers: Option<ChangeModes>,
    pub requires_source_expression_change: Option<bool>,
    pub expression_metadata_state: Option<ResolutionState>,
    pub element_types: Option<Vec<ClassName>>,
    pub supported_axes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Operation implemented by a property handler.
pub enum PropertyHandlerKind {
    Expression,
    Condition,
    Contains,
    TypedValue,
    Wxyz,
    Custom,
}

#[derive(Debug, Clone, PartialEq)]
/// Named arithmetic operator and its registration metadata.
pub struct Operator {
    pub sign: String,
    pub priority: Priority,
    pub key: Option<String>,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Arithmetic implementation for operand and return classes.
pub struct Operation {
    pub operator_sign: String,
    pub left: ClassName,
    pub right: ClassName,
    pub return_type: ClassName,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Difference operation registered for one Java value class.
pub struct Difference {
    pub input_type: ClassName,
    pub return_type: ClassName,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
/// Java class node and direct hierarchy relationships.
pub struct Class {
    pub name: ClassName,
    pub binary_name: String,
    pub kind: ClassKind,
    pub super_class: Option<ClassName>,
    pub interfaces: Vec<ClassName>,
    pub component_type: Option<ClassName>,
    pub container_element_type: Option<ClassName>,
    /// Methods declared directly by this class. `None` means metadata is
    /// unavailable (for example, an older snapshot); `Some(empty)` means
    /// metadata was available and no methods were declared.
    pub methods: Option<Vec<ClassMethod>>,
    pub provider: Option<Addon>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One exact Java method signature returned by `Class.getDeclaredMethods()`.
pub struct ClassMethod {
    pub name: String,
    pub parameter_types: Vec<ClassName>,
    pub return_type: ClassName,
    pub is_static: bool,
}

#[derive(Debug, Clone, PartialEq)]
/// Server-provided Skript aliases and their normalized targets.
pub struct AliasRegistry {
    pub aliases: BTreeMap<String, usize>,
    pub targets: Vec<AliasTarget>,
}

#[derive(Debug, Clone, PartialEq)]
/// Resolved target composed from one or more alias items.
pub struct AliasTarget {
    pub amount: i32,
    pub all: bool,
    pub types: Vec<AliasItem>,
}

#[derive(Debug, Clone, PartialEq)]
/// One material or item component of an alias target.
pub struct AliasItem {
    pub material: String,
    pub minecraft_id: Option<String>,
    pub durability: i32,
    pub plain: bool,
    pub alias: bool,
    pub block_values: Option<Value>,
    pub item_meta: Option<Value>,
}
