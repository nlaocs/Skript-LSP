//! Public semantic contracts produced by CoreLibrary and editable by other addons.
//!
//! Unlike owner-protected metadata, public records are shared candidate state.
//! An addon can deserialize a supported schema version, edit it, and return the
//! changed record from its transform hook. The source text is not rewritten.

use serde::{Deserialize, Serialize};

/// Schema ID for a parsed Skript variable name template.
pub const VARIABLE_SCHEMA_ID: &str = "nlaocs.skript.variable";
/// Version of [`VariableData`]. Unknown versions must not be silently interpreted.
pub const VARIABLE_SCHEMA_VERSION: u32 = 1;

/// Name and scope of a variable reference, not its runtime value.
///
/// Type and multiplicity remain on the enclosing Expression candidate. `name`
/// preserves name-template text (including `::*` and escaped `%%`), without
/// surrounding braces or the local `_` prefix. Embedded expressions refer to
/// the candidate's children in source order. No server values are evaluated.
///
/// # Examples
/// ```
/// use core_library::public_data::{VariableData, VariableNamePart, VariableScope};
/// let mut data = VariableData {
///     scope: VariableScope::Local,
///     name: vec![VariableNamePart::Text { text: "money".into() }],
/// };
/// data.scope = VariableScope::Global;
/// let json = serde_json::to_string(&data).unwrap();
/// assert_eq!(serde_json::from_str::<VariableData>(&json).unwrap(), data);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableData {
    /// Local/global lookup policy; this does not identify a live server variable.
    pub scope: VariableScope,
    /// Literal name fragments and references to embedded child Expressions.
    pub name: Vec<VariableNamePart>,
}

/// Skript variable lookup scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableScope {
    /// An underscore-prefixed local variable.
    Local,
    /// A variable without a local prefix.
    Global,
}

/// One fragment of a variable name template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VariableNamePart {
    /// Source spelling outside `%...%`; escaped `%%` is retained.
    Text {
        /// Name fragment without braces or the local prefix.
        text: String,
    },
    /// A nested Expression, parsed once through the host and retained as a child.
    Expression {
        /// Zero-based index into the enclosing Expression's children.
        #[serde(rename = "childIndex")]
        child_index: u32,
    },
}
