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
pub struct Addon {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionState {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplicity {
    Single,
    Multiple,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeMode {
    Add,
    Set,
    Remove,
    RemoveAll,
    Delete,
    Reset,
}

pub type ChangeModes = BTreeMap<ChangeMode, Vec<ClassName>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Priority {
    pub after: Vec<Priority>,
    pub before: Vec<Priority>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
pub struct Pattern {
    pub source: String,
    pub parsed: ParseResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Experiment {
    pub code_name: String,
    pub phase: String,
    pub known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentalSyntax {
    pub required: Vec<Experiment>,
    pub disallowed: Vec<Experiment>,
    pub error_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnHandler {
    pub return_value_type: Option<ClassName>,
    pub single_return_value: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryValidator {
    pub entry_data: Vec<EntryData>,
}

#[derive(Debug, Clone, PartialEq)]
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
pub enum NodeType {
    Simple,
    Section,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
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
pub struct Event {
    pub common: CommonSyntax,
    pub reference_events: Vec<ClassName>,
    pub event_values: Vec<EventValue>,
    pub cancellable: bool,
    pub has_on_prefix: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub common: CommonSyntax,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub common: CommonSyntax,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    pub common: CommonSyntax,
    pub return_type: Option<ClassName>,
    pub section_expression: bool,
    pub return_type_multiplicity: Option<Multiplicity>,
    pub return_type_multiplicity_state: ResolutionState,
    pub accepted_changers: Option<ChangeModes>,
    pub accepted_changers_state: ResolutionState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub common: CommonSyntax,
    pub loop_section: bool,
    pub effect_section: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Structure {
    pub common: CommonSyntax,
    pub entry_validator: Option<EntryValidator>,
    pub node_type: Option<NodeType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct Noun {
    pub key: String,
    pub value: Option<String>,
    pub singular: String,
    pub plural: String,
    pub gender: i32,
    pub gender_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
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
    pub default_expression_class: Option<ClassName>,
    pub has_parser: bool,
    pub has_serializer: bool,
    pub has_supplier: bool,
    pub properties: Vec<String>,
    pub before: Vec<TypeCodeName>,
    pub after: Vec<TypeCodeName>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParameterModifier {
    Optional,
    Keyed,
    Range { min: Value, max: Value },
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionParameter {
    pub name: String,
    pub parameter_type: ClassName,
    pub modifiers: Vec<ParameterModifier>,
    pub single: bool,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Converter {
    pub from: ClassName,
    pub to: ClassName,
    pub flags: i32,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
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
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
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
pub enum PropertyHandlerKind {
    Expression,
    Condition,
    Contains,
    TypedValue,
    Wxyz,
    Custom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Operator {
    pub sign: String,
    pub priority: Priority,
    pub key: Option<String>,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
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
pub struct Difference {
    pub input_type: ClassName,
    pub return_type: ClassName,
    pub registration_order: usize,
    pub addon: Addon,
    pub registration_id: RegistrationId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: ClassName,
    pub binary_name: String,
    pub kind: ClassKind,
    pub super_class: Option<ClassName>,
    pub interfaces: Vec<ClassName>,
    pub component_type: Option<ClassName>,
    pub provider: Option<Addon>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AliasRegistry {
    pub aliases: BTreeMap<String, usize>,
    pub targets: Vec<AliasTarget>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AliasTarget {
    pub amount: i32,
    pub all: bool,
    pub types: Vec<AliasItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AliasItem {
    pub material: String,
    pub minecraft_id: Option<String>,
    pub durability: i32,
    pub plain: bool,
    pub alias: bool,
    pub block_values: Option<Value>,
    pub item_meta: Option<Value>,
}
