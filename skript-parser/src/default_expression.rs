//! Omitted typed arguments resolved by parser environments and WASM addons.

use std::collections::BTreeMap;

use syntax_pattern_parser::syntax::{PatternTypeExpr, Span as PatternSpan};
use syntaxes::{ClassName, Multiplicity, Type};

use crate::{
    ExpressionEffects, ExpressionExpectedType, ExpressionParseContext, ExpressionPublicData,
    MatchSpan, RegisteredSyntaxIdentity, SemanticDiagnostic,
};

/// A static rejection is distinct from missing or insufficient provider knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DefaultExpressionFailureKind {
    Rejected,
    Unresolved,
}

/// A small, snapshot-bound reference used to explain a default Expression.
///
/// Providers define the role (for example `event-value`) and supply available
/// identities. No Catalog documents or record bodies are copied into the AST.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefaultExpressionCatalogReference {
    pub role: String,
    pub definition_id: Option<String>,
    pub registration_id: Option<String>,
    pub source_digest: Option<String>,
    pub snapshot_id: Option<String>,
    pub document: Option<String>,
    pub index: Option<u64>,
}

/// One omitted, non-nullable capture and one requested Type alternative.
///
/// Environments receive the exact syntax and Type identities rather than
/// recognizing implementation class names. The mapped span is a zero-width
/// insertion anchor; there is no generated source string to parse.
pub struct DefaultExpressionRequest<'a> {
    pub syntax: RegisteredSyntaxIdentity<'a>,
    pub capture_index: usize,
    pub pattern_span: PatternSpan,
    pub expression: &'a PatternTypeExpr,
    pub expected_type: &'a ExpressionExpectedType,
    pub value_type: &'a Type,
    pub span: &'a MatchSpan,
    pub context: &'a ExpressionParseContext,
}

/// A provider's result before ordinary type, multiplicity and flag validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultExpressionResolution {
    pub provider_id: String,
    pub component_id: String,
    pub return_type: ClassName,
    pub multiplicity: Multiplicity,
    pub is_literal: bool,
    /// Time state actually supported by this initialized default.
    pub time: i32,
    pub reason: String,
    pub catalog_references: Vec<DefaultExpressionCatalogReference>,
    pub metadata: BTreeMap<String, String>,
    pub public_data: Vec<ExpressionPublicData>,
    pub effects: Option<ExpressionEffects>,
}

/// The provider must distinguish invalid context from unavailable knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultExpressionDecision {
    Resolved(Box<DefaultExpressionResolution>),
    Rejected {
        reason: String,
        diagnostics: Vec<SemanticDiagnostic>,
    },
    Unresolved {
        reason: String,
    },
}

/// Provenance of an implicit child in the shared Expression tree.
///
/// The node itself retains resolved type, multiplicity, metadata, public data
/// and mapped anchor. This record describes the omitted capture and the
/// provider's evidence; it never represents text present in the source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefaultExpressionInfo {
    pub capture_index: usize,
    pub pattern_span: PatternSpan,
    pub expression: PatternTypeExpr,
    /// Exact placeholder spelling from the registered pattern, not source input.
    pub expression_source: String,
    pub requested_type: ExpressionExpectedType,
    pub type_definition_id: String,
    pub type_registration_id: String,
    pub provider_id: String,
    pub component_id: String,
    pub reason: String,
    /// Context references; enclosing Section trees are not copied into each child.
    pub event_classes: Vec<ClassName>,
    pub section_scope_ids: Vec<u64>,
    /// Zero-width source anchor, also retained when this node becomes a scope summary.
    pub anchor: crate::MappedSpan,
    pub catalog_references: Vec<DefaultExpressionCatalogReference>,
    pub is_literal: bool,
    pub time: i32,
}
