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

/// Parser-confirmed interpolation delimiters, relative UTF-8 byte offsets into
/// the enclosing Expression source. They are not inferred from a trimmed child.
pub const INTERPOLATION_SCHEMA_ID: &str = "nlaocs.skript.interpolation";
pub const INTERPOLATION_SCHEMA_VERSION: u32 = 1;

/// Semantic roles for selected native pattern slots. Indices are references,
/// never source ranges: consumers must bind them to current sealed geometry.
pub const CAPTURE_ROLES_SCHEMA_ID: &str = "nlaocs.skript.capture-roles";
pub const CAPTURE_ROLES_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRolesData {
    pub captures: Vec<CaptureRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRole {
    #[serde(rename = "captureIndex")]
    pub capture_index: u64,
    pub role: String,
}

/// Replace this schema only; unrelated public records remain available to addons.
pub(crate) fn mark_expression_capture(
    payload: &mut crate::nlaocs::skript_parser_addon::types::RegisteredExpressionPayload,
    capture_index: u64,
) {
    use crate::nlaocs::skript_parser_addon::types::ExpressionPublicData;
    payload
        .public_data
        .retain(|record| record.schema_id != CAPTURE_ROLES_SCHEMA_ID);
    payload.public_data.push(ExpressionPublicData {
        schema_id: CAPTURE_ROLES_SCHEMA_ID.into(),
        schema_version: CAPTURE_ROLES_SCHEMA_VERSION,
        json: serde_json::to_string(&CaptureRolesData {
            captures: vec![CaptureRole {
                capture_index,
                role: "expression".into(),
            }],
        })
        .expect("capture-role data is serializable"),
    });
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpolationData {
    pub expressions: Vec<InterpolationExpression>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpolationExpression {
    #[serde(rename = "childIndex")]
    pub child_index: u32,
    /// Offset of the opening percent sign, excluding escaped `%%`.
    pub opening: u64,
    /// Offset of the closing percent sign; the inner source ends here.
    pub closing: u64,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_role_schema_serializes_slot_references_without_source_ranges() {
        let value = CaptureRolesData {
            captures: vec![CaptureRole {
                capture_index: 7,
                role: "expression".into(),
            }],
        };
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"captures":[{"captureIndex":7,"role":"expression"}]})
        );
        assert_eq!(
            serde_json::from_value::<CaptureRolesData>(json).unwrap(),
            value
        );
    }
}
