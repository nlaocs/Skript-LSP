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

/// Actual static variable-name identity, separate from the source template v1.
pub const VARIABLE_IDENTITY_SCHEMA_ID: &str = "nlaocs.skript.variable-identity";
pub const VARIABLE_IDENTITY_SCHEMA_VERSION: u32 = 1;

/// A definite name is not a claim about the server's configurable case policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableIdentityData {
    pub scope: VariableScope,
    pub resolution: VariableIdentityResolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Potential alias under the supported insensitive policy, not a canonical key.
    /// Absent for dynamic or non-ASCII names: Java Unicode mappings depend on runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_key: Option<String>,
    pub list: bool,
    pub case_policy: VariableCasePolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_name: Option<String>,
    /// Optional exact UTF-8 name range relative to this variable expression.
    /// Absent for templates, list families, parameters and unproven spellings.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name_range: Option<VariableNameRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableNameRange {
    pub start: u64,
    pub end: u64,
    pub grammar: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableIdentityResolution {
    Static,
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariableCasePolicy {
    Unknown,
    Sensitive,
    EnglishInsensitive,
}

impl VariableIdentityData {
    /// The template already has Skript's whitespace/local-prefix normalization.
    /// Static VariableString names collapse escaped percent pairs at runtime.
    /// No runtime case setting is present in the current SSG contract.
    pub fn from_template(scope: VariableScope, parts: &[VariableNamePart], list: bool) -> Self {
        let static_text = parts
            .iter()
            .map(|part| match part {
                VariableNamePart::Text { text } => Some(text.as_str()),
                VariableNamePart::Expression { .. } => None,
            })
            .collect::<Option<Vec<_>>>();
        let name = static_text.map(|parts| parts.concat().replace("%%", "%"));
        Self::from_resolved_name(scope, name, list)
    }

    /// A Function parameter is an already initialized runtime name, not VariableString source.
    pub fn from_static_name(scope: VariableScope, name: String, list: bool) -> Self {
        Self::from_resolved_name(scope, Some(name), list)
    }

    fn from_resolved_name(scope: VariableScope, name: Option<String>, list: bool) -> Self {
        let alias_key = name
            .as_ref()
            .filter(|name| name.is_ascii())
            .map(|name| name.to_ascii_lowercase());
        Self {
            scope,
            resolution: if name.is_some() {
                VariableIdentityResolution::Static
            } else {
                VariableIdentityResolution::Dynamic
            },
            name,
            alias_key,
            list,
            case_policy: VariableCasePolicy::Unknown,
            canonical_name: None,
            name_range: None,
        }
    }
}

#[cfg(test)]
mod variable_identity_tests {
    use super::*;
    #[test]
    fn static_identity_preserves_exact_runtime_name_without_selecting_case_policy() {
        let value = VariableIdentityData::from_template(
            VariableScope::Local,
            &[VariableNamePart::Text {
                text: "Money%%Value::1".into(),
            }],
            false,
        );
        assert_eq!(value.name.as_deref(), Some("Money%Value::1"));
        assert_eq!(value.alias_key.as_deref(), Some("money%value::1"));
        assert_eq!(value.case_policy, VariableCasePolicy::Unknown);
        assert_eq!(value.canonical_name, None);
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json["casePolicy"], "unknown");
        assert_eq!(json["resolution"], "static");
        assert!(json.get("canonicalName").is_none());
    }
    #[test]
    fn initialized_parameter_names_are_not_variable_string_source() {
        let value =
            VariableIdentityData::from_static_name(VariableScope::Local, "arg%%Name".into(), false);
        assert_eq!(value.name.as_deref(), Some("arg%%Name"));
        assert_eq!(value.alias_key.as_deref(), Some("arg%%name"));
        assert_eq!(value.canonical_name, None);
    }
    #[test]
    fn dynamic_names_and_unknown_unicode_mapping_do_not_gain_guessed_aliases() {
        let dynamic = VariableIdentityData::from_template(
            VariableScope::Global,
            &[
                VariableNamePart::Text {
                    text: "value::".into(),
                },
                VariableNamePart::Expression { child_index: 0 },
            ],
            false,
        );
        assert_eq!(dynamic.resolution, VariableIdentityResolution::Dynamic);
        assert_eq!(dynamic.name, None);
        assert_eq!(dynamic.alias_key, None);
        let unicode = VariableIdentityData::from_template(
            VariableScope::Local,
            &[VariableNamePart::Text {
                text: "\u{03a3}::*".into(),
            }],
            true,
        );
        assert_eq!(unicode.name.as_deref(), Some("\u{03a3}::*"));
        assert_eq!(unicode.alias_key, None);
        assert!(unicode.list);
    }
}

/// Intrinsic evaluation footprint. Children are evaluated separately by consumers.
/// This owner-protected witness requires the native semantic contributor proof;
/// a retained metadata entry alone cannot authenticate a rewritten candidate.
pub const LOCAL_EFFECTS_METADATA_KEY: &str = "skript.semantic.local-effects";
pub(crate) fn preserve_local_effects() -> crate::nlaocs::skript_parser_addon::types::MetadataEntry {
    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
        key: LOCAL_EFFECTS_METADATA_KEY.into(),
        value: r#"{"version":1,"kind":"preserve"}"#.into(),
        owner_component_id: None,
    }
}

pub(crate) fn publish_metadata(
    metadata: &mut Vec<crate::nlaocs::skript_parser_addon::types::MetadataEntry>,
    entry: crate::nlaocs::skript_parser_addon::types::MetadataEntry,
) {
    metadata.retain(|old| {
        old.key != entry.key
            || old
                .owner_component_id
                .as_deref()
                .is_some_and(|owner| owner != crate::COMPONENT_ID)
    });
    metadata.push(entry);
}
/// Ordered early capture evaluations. Unknown/unlisted operands remain isolated.
pub const OPERAND_EVALUATION_METADATA_KEY: &str = "skript.semantic.operand-evaluation";
pub(crate) fn operand_evaluation(
    prefix: &[u64],
) -> crate::nlaocs::skript_parser_addon::types::MetadataEntry {
    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
        key: OPERAND_EVALUATION_METADATA_KEY.into(),
        value: serde_json::json!({"version":1,"prefix":prefix}).to_string(),
        owner_component_id: None,
    }
}
pub const VALUE_RESULT_METADATA_KEY: &str = "skript.semantic.value-result";
pub(crate) fn non_empty_value_result() -> crate::nlaocs::skript_parser_addon::types::MetadataEntry {
    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
        key: VALUE_RESULT_METADATA_KEY.into(),
        value: r#"{"version":1,"kind":"non-empty"}"#.into(),
        owner_component_id: None,
    }
}
/// Actual inherited Expression.beforeChange clone/slot path, without an override.
pub const CHANGE_INPUT_METADATA_KEY: &str = "skript.semantic.change-input";
pub(crate) fn default_clone_change_input()
-> crate::nlaocs::skript_parser_addon::types::MetadataEntry {
    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
        key: CHANGE_INPUT_METADATA_KEY.into(),
        value: r#"{"version":1,"kind":"default-clone"}"#.into(),
        owner_component_id: None,
    }
}
/// This is a lookup observation boundary, not a preservation promise: Variable
/// can refresh a stored Player binding and execute requested-type converters.
pub const VARIABLE_READ_METADATA_KEY: &str = "skript.semantic.variable-read";
pub(crate) fn variable_read(
    requested_types: &[String],
    local: bool,
) -> crate::nlaocs::skript_parser_addon::types::MetadataEntry {
    let version = crate::runtime::current().and_then(|profile| profile.skript_version);
    let missing_is_empty = static_variable_missing_is_empty(version.as_deref(), local);
    let mut contract = serde_json::json!({"version":1,"requestedTypes":requested_types,
        "mayRefreshPlayer":true,"noRefreshTypes":crate::catalog::identity_delta_classes()});
    if missing_is_empty {
        contract["version"] = if requested_types==["java.lang.Object"] {3}else{2}.into();
        contract["missingValue"] = "empty".into();
    }
    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
        key: VARIABLE_READ_METADATA_KEY.into(),
        value:contract.to_string(),
        owner_component_id:None,
    }
}

// Only the fixed-name scalar primitive calls variable_read. In audited 2.15.4,
// VariableString.newInstance returns LiteralString for a name with no embedded
// Expressions. That constructor retains MESSAGE mode; getDefaultVariableNames
// returns no candidates before considering isSimple. Variable.getRaw therefore
// returns the stored value or null, without a default-candidate lookup. Interpolated
// names and unreviewed versions must not inherit this missing-value contract.
fn static_variable_missing_is_empty(version: Option<&str>, local: bool) -> bool {
    let _=local;
    version == Some("2.15.4")
}

/// Initialized default source, not a generated source expression or a live value.
pub const DEFAULT_SOURCE_SCHEMA_ID: &str = "nlaocs.skript.default-source";
pub const DEFAULT_SOURCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultSourceData {
    pub source_kind: DefaultSourceKind,
    pub implementation_class: String,
    pub target_class: String,
    pub time: i32,
    pub event_values: Vec<DefaultEventValueSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultSourceKind {
    Literal,
    EventValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultEventValueSource {
    pub event_class: String,
    pub value_class: String,
    pub time: i32,
    pub registration_id: String,
}

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
    fn fixed_name_missing_contract_is_scoped_to_the_audited_standard_adapter() {
        assert!(static_variable_missing_is_empty(Some("2.15.4"), true));
        assert!(static_variable_missing_is_empty(Some("2.15.4"), false));
        for version in [None,Some("2.15.3"),Some("2.15.5"),Some("2.16"),Some("2.15.4-custom"),Some("unknown")] {
            assert!(!static_variable_missing_is_empty(version, true));
        }
    }
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

/// Parser/init context effects, independent of runtime local-value effects.
pub const EVALUATION_CONTEXT_METADATA_KEY: &str = "skript.semantic.evaluation-context";
pub(crate) fn preserve_evaluation_context()
-> crate::nlaocs::skript_parser_addon::types::MetadataEntry {
    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
        key: EVALUATION_CONTEXT_METADATA_KEY.into(),
        value: r#"{"version":1,"kind":"preserve"}"#.into(),
        owner_component_id: None,
    }
}
/// Audited standard ReturnableTrigger -> ScriptFunction -> value.getArray entry.
pub const SCRIPT_RETURN_ENTRY: &str = "skript-2.15.4.script-return-value-entry.v1";
