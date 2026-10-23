//! Guest-side helpers for the host's complete, read-only SSG Catalog view.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{LazyLock, RwLock},
};

use serde::{Deserialize, Serialize};

use crate::nlaocs::skript_parser_addon::types::{MetadataEntry, MetadataResolutionState};

pub(crate) const CHANGE_CONTRACT_METADATA_KEY: &str = "change-contract";
const CHANGE_CONTRACT_SCHEMA_VERSION: u32 = 1;
const MAX_CACHED_CHANGE_CONTRACTS: usize = 1_024;
#[cfg(target_arch = "wasm32")]
const MAX_SOURCE_RECORDS_PER_LOOKUP: usize = 256;
#[cfg(target_arch = "wasm32")]
const MAX_SOURCE_BYTES_PER_LOOKUP: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub(crate) enum ChangeContract {
    Resolved {
        modes: BTreeMap<String, Vec<AcceptedChangeType>>,
    },
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeContractEnvelope {
    schema_version: u32,
    subject_id: String,
    contract: ChangeContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedChangeType {
    pub class_name: String,
    pub multiple: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeRelation {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComparatorContract {
    pub relation: TypeRelation,
    pub supports_ordering: Option<bool>,
    pub supports_inversion: Option<bool>,
    pub registration_id: Option<String>,
    pub reversed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PropertyHandlerContract {
    pub property_registration_id: String,
    pub property_name: String,
    pub input_class: String,
    pub handler_class: String,
    pub handler_kind: String,
    pub element_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeSerializationContract {
    pub type_class: String,
    pub has_serializer: bool,
    pub serialize_as: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogRecordReference {
    pub source_digest: String,
    pub snapshot_id: String,
    pub document: String,
    pub index: u64,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventValueOption {
    pub source_record: Option<CatalogRecordReference>,
    pub event_class: String,
    pub value_class: String,
    pub time: i32,
    pub registration_id: String,
    pub patterns: Vec<String>,
    pub excludes: Vec<String>,
    pub exclude_error_message: Option<String>,
    pub resolution_order: u64,
    pub registration_order: Option<u64>,
    pub accepted_changers: BTreeMap<String, Vec<AcceptedChangeType>>,
    pub context_dependent: Option<bool>,
    pub has_custom_input_validator: Option<bool>,
    pub has_custom_event_validator: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DifferenceOption {
    pub input_class: String,
    pub return_class: String,
    pub registration_id: String,
    pub registration_order: u64,
    pub hierarchy_distance: Option<u64>,
}

impl AcceptedChangeType {
    pub fn display_name(&self) -> String {
        format!(
            "{}{}",
            self.class_name,
            if self.multiple { "[]" } else { "" }
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionSourceRecord {
    accepted_changers: Option<BTreeMap<String, Vec<String>>>,
    accepted_changers_state: ResolutionState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventValueSourceRecord {
    accepted_changers: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ResolutionState {
    Resolved,
    Unresolved,
}

static CHANGE_CONTRACTS: LazyLock<RwLock<HashMap<String, Option<ChangeContract>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static EVENT_VALUE_LOCAL_READS: LazyLock<RwLock<HashMap<String, bool>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static STANDARD_CONDITION_SOURCES: LazyLock<RwLock<HashMap<String, bool>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static IMMUTABLE_RUNTIME_CLASSES: LazyLock<RwLock<HashMap<String, Vec<String>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static STANDARD_RUNTIME_SOURCES: LazyLock<RwLock<HashMap<String, bool>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static EXPERIMENTS: LazyLock<RwLock<HashMap<String, Vec<CatalogExperiment>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CatalogExperiment {
    pub code_name: String,
    pub phase: String,
    pub known: bool,
}

struct SourceRecord {
    document: String,
    bytes: Vec<u8>,
}

pub(crate) fn experiments() -> Result<Vec<CatalogExperiment>, String> {
    let digest = catalog_source_digest()?;
    if let Some(cached) = EXPERIMENTS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&digest)
        .cloned()
    {
        return Ok(cached);
    }
    let experiments = catalog_experiments()?;
    EXPERIMENTS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(digest, experiments.clone());
    Ok(experiments)
}

#[cfg(test)]
fn collect_experiments(value: &serde_json::Value, output: &mut Vec<CatalogExperiment>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_experiments(value, output);
            }
        }
        serde_json::Value::Object(object) => {
            if let Some(experimental) = object.get("experimentalSyntax").and_then(|v| v.as_object())
            {
                for key in ["required", "disallowed"] {
                    for experiment in experimental
                        .get(key)
                        .and_then(|value| value.as_array())
                        .into_iter()
                        .flatten()
                    {
                        let Some(experiment) = experiment.as_object() else {
                            continue;
                        };
                        let (Some(code_name), Some(phase)) = (
                            experiment.get("codeName").and_then(|value| value.as_str()),
                            experiment.get("phase").and_then(|value| value.as_str()),
                        ) else {
                            continue;
                        };
                        output.push(CatalogExperiment {
                            code_name: code_name.to_owned(),
                            phase: phase.to_owned(),
                            known: experiment
                                .get("known")
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false),
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(target_arch = "wasm32")]
fn catalog_experiments() -> Result<Vec<CatalogExperiment>, String> {
    crate::nlaocs::skript_parser_addon::catalog_data::experiments()
        .map(|experiments| {
            experiments
                .into_iter()
                .map(|experiment| CatalogExperiment {
                    code_name: experiment.code_name,
                    phase: experiment.phase,
                    known: experiment.known,
                })
                .collect()
        })
        .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn catalog_experiments() -> Result<Vec<CatalogExperiment>, String> {
    Ok(Vec::new())
}

pub(crate) fn expression_change_contract(
    registration_id: &str,
) -> Result<Option<ChangeContract>, String> {
    let cache_key = format!("{}\0{registration_id}", catalog_source_digest()?);
    if let Some(cached) = CHANGE_CONTRACTS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&cache_key)
        .cloned()
    {
        return Ok(cached);
    }

    let records = registration_records(registration_id)?;
    let contracts = records
        .iter()
        .filter_map(|record| match record.document.as_str() {
            "Expressions.json" => Some(parse_change_contract(&record.bytes)),
            "EventValues.json" => Some(parse_event_value_change_contract(&record.bytes)),
            _ => None,
        })
        .collect::<Result<Vec<_>, _>>()?;
    let contract = match contracts.as_slice() {
        [] => None,
        [first, rest @ ..] if rest.iter().all(|contract| contract == first) => Some(first.clone()),
        _ => {
            return Err(format!(
                "registration ID {registration_id} has conflicting change data"
            ));
        }
    };
    let mut cache = CHANGE_CONTRACTS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_CACHED_CHANGE_CONTRACTS && !cache.contains_key(&cache_key) {
        cache.clear();
    }
    cache.insert(cache_key, contract.clone());
    Ok(contract)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn type_change_contract(class_name: &str) -> Result<Option<ChangeContract>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::change_contract_for_type(class_name)
        .map_err(|error| error.message)
        .map(|contract| {
            contract.map(|contract| ChangeContract::Resolved {
                modes: if contract.has_changer {
                    contract
                        .modes
                        .into_iter()
                        .map(|mode| {
                            (
                                mode.mode,
                                mode.accepted_types
                                    .into_iter()
                                    .map(accepted_change_type)
                                    .collect(),
                            )
                        })
                        .collect()
                } else {
                    BTreeMap::new()
                },
            })
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn type_change_contract(_class_name: &str) -> Result<Option<ChangeContract>, String> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn type_serialization_contract(
    class_name: &str,
) -> Result<Option<TypeSerializationContract>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::serialization_contract_for_type(class_name)
        .map_err(|error| error.message)
        .map(|contract| {
            contract.map(|contract| TypeSerializationContract {
                type_class: contract.type_class,
                has_serializer: contract.has_serializer,
                serialize_as: contract.serialize_as,
            })
        })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn event_values_for(event_class: &str) -> Result<Vec<EventValueOption>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::event_values_for(event_class)
        .map_err(|error| error.message)
        .map(|values| values.into_iter().map(event_value_option).collect())
}

pub(crate) fn type_for_class(
    class_name: &str,
) -> Result<Option<crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::nlaocs::skript_parser_addon::catalog_data::type_for_class(class_name)
            .map_err(|error| error.message)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = class_name;
        Ok(None)
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn event_values_for_input(
    event_class: &str,
    input: &str,
) -> Result<Vec<EventValueOption>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::event_values_for_input(event_class, input)
        .map_err(|error| error.message)
        .map(|values| values.into_iter().map(event_value_option).collect())
}

#[cfg(target_arch = "wasm32")]
fn event_value_option(
    value: crate::nlaocs::skript_parser_addon::catalog_data::EventValueOption,
) -> EventValueOption {
    EventValueOption {
        source_record: value.source_record.map(|record| CatalogRecordReference {
            source_digest: record.source_digest,
            snapshot_id: record.snapshot_id,
            document: record.document,
            index: record.index,
            byte_length: record.byte_length,
        }),
        event_class: value.event_class,
        value_class: value.value_class,
        time: value.time,
        registration_id: value.registration_id,
        patterns: value.patterns,
        excludes: value.excludes,
        exclude_error_message: value.exclude_error_message,
        resolution_order: value.resolution_order,
        registration_order: value.registration_order,
        accepted_changers: value
            .accepted_changers
            .into_iter()
            .map(|mode| {
                (
                    mode.mode,
                    mode.accepted_types
                        .into_iter()
                        .map(accepted_change_type)
                        .collect(),
                )
            })
            .collect(),
        context_dependent: value.context_dependent,
        has_custom_input_validator: value.has_custom_input_validator,
        has_custom_event_validator: value.has_custom_event_validator,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn event_values_for(_event_class: &str) -> Result<Vec<EventValueOption>, String> {
    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn event_values_for_input(
    _event_class: &str,
    _input: &str,
) -> Result<Vec<EventValueOption>, String> {
    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn type_serialization_contract(
    _class_name: &str,
) -> Result<Option<TypeSerializationContract>, String> {
    Ok(None)
}

pub(crate) fn change_contract_metadata(
    subject_id: &str,
    contract: &ChangeContract,
) -> Result<MetadataEntry, String> {
    Ok(MetadataEntry {
        key: CHANGE_CONTRACT_METADATA_KEY.to_owned(),
        value: serde_json::to_string(&ChangeContractEnvelope {
            schema_version: CHANGE_CONTRACT_SCHEMA_VERSION,
            subject_id: subject_id.to_owned(),
            contract: contract.clone(),
        })
        .map_err(|error| format!("could not encode change contract: {error}"))?,
        owner_component_id: None,
    })
}

pub(crate) fn change_contract_from_metadata(
    metadata: &[MetadataEntry],
    expected_subject_id: &str,
) -> Result<Option<ChangeContract>, String> {
    let contracts = metadata
        .iter()
        .filter(|entry| entry.key == CHANGE_CONTRACT_METADATA_KEY)
        .map(|entry| {
            let envelope =
                serde_json::from_str::<ChangeContractEnvelope>(&entry.value).map_err(|error| {
                    let owner = entry
                        .owner_component_id
                        .as_deref()
                        .unwrap_or("unknown addon");
                    format!("{owner} published an invalid change contract: {error}")
                })?;
            if envelope.schema_version != CHANGE_CONTRACT_SCHEMA_VERSION {
                return Err(format!(
                    "change contract schema {} is unsupported; expected {}",
                    envelope.schema_version, CHANGE_CONTRACT_SCHEMA_VERSION
                ));
            }
            if envelope.subject_id != expected_subject_id {
                return Err(format!(
                    "change contract targets {:?}, but it is attached to {:?}",
                    envelope.subject_id, expected_subject_id
                ));
            }
            Ok(envelope.contract)
        })
        .collect::<Result<Vec<_>, _>>()?;
    match contracts.as_slice() {
        [] => Ok(None),
        [first, rest @ ..] if rest.iter().all(|contract| contract == first) => {
            Ok(Some(first.clone()))
        }
        _ => Err("multiple addons published conflicting change contracts".to_owned()),
    }
}

pub(crate) fn property_change_contract(
    options: &[crate::nlaocs::skript_parser_addon::types::RegisteredExpressionPropertyOption],
    source: Option<&crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild>,
) -> ChangeContract {
    // Skript 2.15.4 gates the whole PropertyMap when any handler needs source mutation,
    // then unions every selected handler's changer types once that gate succeeds.
    if options
        .iter()
        .any(|option| option.requires_source_expression_change == Some(true))
    {
        let Some(source) = source else {
            return ChangeContract::Unresolved;
        };
        let mut accepted_source = false;
        let mut unknown_source = false;
        for option in options {
            match source_accepts_set_type(source, &option.input_class) {
                Ok(Some(true)) => accepted_source = true,
                Ok(Some(false)) => {}
                Ok(None) | Err(_) => unknown_source = true,
            }
        }
        if !accepted_source {
            return if unknown_source {
                ChangeContract::Unresolved
            } else {
                ChangeContract::Resolved {
                    modes: BTreeMap::new(),
                }
            };
        }
    }

    let mut modes = BTreeMap::<String, Vec<AcceptedChangeType>>::new();
    for option in options {
        if option.accepted_changers_state != Some(MetadataResolutionState::Resolved) {
            return ChangeContract::Unresolved;
        }
        for accepted in &option.accepted_changers {
            let values = modes.entry(accepted.mode.clone()).or_default();
            values.extend(
                accepted
                    .accepted_types
                    .iter()
                    .map(|class_name| accepted_change_type(class_name.clone())),
            );
        }
    }
    for values in modes.values_mut() {
        values.sort_by(|left, right| {
            (&left.class_name, left.multiple).cmp(&(&right.class_name, right.multiple))
        });
        values.dedup();
    }
    ChangeContract::Resolved { modes }
}

fn source_accepts_set_type(
    source: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild,
    input_class: &str,
) -> Result<Option<bool>, String> {
    let subject_id = source
        .registration_id
        .as_deref()
        .or(source.parser_id.as_deref());
    let contract = match subject_id
        .map(|subject_id| change_contract_from_metadata(&source.metadata, subject_id))
        .transpose()?
        .flatten()
    {
        Some(contract) => Some(contract),
        None => match source.registration_id.as_deref() {
            Some(registration_id) => expression_change_contract(registration_id)?,
            None => None,
        },
    };
    let Some(ChangeContract::Resolved { modes }) = contract else {
        return Ok(None);
    };
    let Some(accepted) = modes.get("SET") else {
        return Ok(Some(false));
    };
    let mut relation_unknown = false;
    for target in accepted {
        match is_class_assignable(input_class, &target.class_name)? {
            TypeRelation::Compatible => return Ok(Some(true)),
            TypeRelation::Incompatible => {}
            TypeRelation::Unknown => relation_unknown = true,
        }
    }
    Ok((!relation_unknown).then_some(false))
}

/// Only the audited standard getter implementation is certified. This is not
/// inferred from a Type or getter-shaped pattern: an addon getter/converter can
/// execute arbitrary code, including a caller-local write.
pub(crate) fn event_value_preserves_locals(value: &EventValueOption, target_class: &str) -> bool {
    if value.context_dependent != Some(false)
        || value.has_custom_input_validator != Some(false)
        || value.has_custom_event_validator != Some(false)
        || !matches!(
            is_class_assignable(
                value.value_class.trim_end_matches("[]"),
                target_class.trim_end_matches("[]")
            ),
            Ok(TypeRelation::Compatible)
        )
    {
        return false;
    }
    let Ok(digest) = catalog_source_digest() else {
        return false;
    };
    let key = format!(
        "{digest}\0{}\0{}\0{}\0{}",
        value.registration_id, value.event_class, value.value_class, value.time
    );
    if let Some(result) = EVENT_VALUE_LOCAL_READS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .copied()
    {
        return result;
    }
    let result = registration_records(&value.registration_id)
        .ok()
        .is_some_and(|records| {
            let records = records
                .iter()
                .filter(|record| record.document == "EventValues.json")
                .collect::<Vec<_>>();
            records.len() == 1 && standard_event_value_local_read(&records[0].bytes, value)
        });
    let mut cache = EVENT_VALUE_LOCAL_READS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_CACHED_CHANGE_CONTRACTS {
        cache.clear();
    }
    cache.insert(key, result);
    result
}

fn standard_event_value_local_read(_bytes: &[u8], _value: &EventValueOption) -> bool {
    // The current export identifies registration ownership and shape, but does
    // not retain the actual converter/getter implementation. Ownership alone
    // cannot prove that a replacement callback preserves caller-local bindings.
    false
}

/// Authenticate an audited provider family against the exact immutable source.
/// Handler suffix matching alone cannot certify an addon implementation.
pub(crate) fn standard_condition_source(
    candidate: &crate::nlaocs::skript_parser_addon::types::ConditionCandidate,
    expected_class: &str,
) -> bool {
    if !pinned_runtime() || candidate.element_class.as_deref() != Some(expected_class) {
        return false;
    }
    let Ok(digest) = catalog_source_digest() else {
        return false;
    };
    let key = format!(
        "{digest}\0{}\0{}\0{expected_class}\0{}\0{}",
        candidate.registration_id,
        candidate.definition_id,
        candidate.pattern_index,
        candidate.pattern
    );
    if let Some(result) = STANDARD_CONDITION_SOURCES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .copied()
    {
        return result;
    }
    let result = registration_records(&candidate.registration_id)
        .ok()
        .is_some_and(|records| {
            let records = records
                .iter()
                .filter(|record| record.document == "Conditions.json")
                .collect::<Vec<_>>();
            records.len() == 1
                && standard_condition_source_record(&records[0].bytes, candidate, expected_class)
        });
    let mut cache = STANDARD_CONDITION_SOURCES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_CACHED_CHANGE_CONTRACTS {
        cache.clear();
    }
    cache.insert(key, result);
    result
}

fn standard_condition_source_record(
    bytes: &[u8],
    candidate: &crate::nlaocs::skript_parser_addon::types::ConditionCandidate,
    expected_class: &str,
) -> bool {
    let Ok(record) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let Ok(index) = usize::try_from(candidate.pattern_index) else {
        return false;
    };
    record["kind"].as_str() == Some("condition")
        && record["addon"]["name"].as_str() == Some("Skript")
        && record["addon"]["version"].as_str() == Some("2.15.4")
        && record["definitionId"].as_str() == Some(candidate.definition_id.as_str())
        && record["registrationId"].as_str() == Some(candidate.registration_id.as_str())
        && record["elementClass"].as_str() == Some(expected_class)
        && record["patterns"]
            .as_array()
            .and_then(|values| values.get(index))
            .and_then(serde_json::Value::as_str)
            == Some(candidate.pattern.as_str())
}

pub(crate) fn standard_expression_source(
    candidate: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionPayload,
    class: &str,
) -> bool {
    pinned_runtime()
        && candidate.element_class == class
        && standard_syntax_source(
            "Expressions.json",
            "expression",
            &candidate.definition_id,
            &candidate.registration_id,
            class,
            candidate.pattern_index,
            Some(&candidate.pattern),
        )
}

pub(crate) fn standard_effect_source(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
    class: &str,
) -> bool {
    pinned_runtime()
        && candidate.element_class.as_deref() == Some(class)
        && standard_syntax_source(
            "Effects.json",
            "effect",
            &candidate.definition_id,
            &candidate.registration_id,
            class,
            candidate.pattern_index,
            Some(&candidate.pattern),
        )
}
pub(crate) fn standard_section_source(
    candidate: &crate::nlaocs::skript_parser_addon::types::SectionCandidate,
    class: &str,
) -> bool {
    pinned_runtime()
        && candidate.element_class.as_deref() == Some(class)
        && standard_syntax_source(
            "Sections.json",
            "section",
            &candidate.definition_id,
            &candidate.registration_id,
            class,
            candidate.pattern_index,
            None,
        )
}
pub(crate) fn standard_structure_source(
    candidate: &crate::nlaocs::skript_parser_addon::types::StructureCandidate,
    class: &str,
) -> bool {
    pinned_runtime()
        && candidate.element_class.as_deref() == Some(class)
        && standard_syntax_source(
            "Structures.json",
            "structure",
            &candidate.definition_id,
            &candidate.registration_id,
            class,
            candidate.pattern_index,
            Some(&candidate.pattern),
        )
}
fn standard_syntax_source(
    document: &str,
    kind: &str,
    definition: &str,
    registration: &str,
    class: &str,
    index: u64,
    pattern: Option<&str>,
) -> bool {
    let Ok(digest) = catalog_source_digest() else {
        return false;
    };
    let key = serde_json::json!([
        digest,
        document,
        definition,
        registration,
        class,
        index,
        pattern
    ])
    .to_string();
    if let Some(value) = STANDARD_RUNTIME_SOURCES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .copied()
    {
        return value;
    }
    let result = registration_records(registration)
        .ok()
        .is_some_and(|records| {
            let records = records
                .iter()
                .filter(|record| record.document == document)
                .collect::<Vec<_>>();
            let Ok(index) = usize::try_from(index) else {
                return false;
            };
            records.len() == 1
                && serde_json::from_slice::<serde_json::Value>(&records[0].bytes)
                    .ok()
                    .is_some_and(|record| {
                        record["kind"].as_str() == Some(kind)
                            && record["addon"]["name"].as_str() == Some("Skript")
                            && record["addon"]["version"].as_str() == Some("2.15.4")
                            && record["definitionId"].as_str() == Some(definition)
                            && record["registrationId"].as_str() == Some(registration)
                            && record["elementClass"].as_str() == Some(class)
                            && record["patterns"]
                                .as_array()
                                .and_then(|patterns| patterns.get(index))
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|selected| {
                                    pattern.is_none_or(|pattern| selected == pattern)
                                })
                    })
        });
    let mut cache = STANDARD_RUNTIME_SOURCES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_CACHED_CHANGE_CONTRACTS {
        cache.clear();
    }
    cache.insert(key, result);
    result
}

/// The closed final classes are eligible only when actual versioned ClassInfo
/// dispatch facts prove identity clone and absent serialization redirection.
/// Unknown/old type definitions never gain this beforeChange/refresh proof.
pub(crate) fn identity_delta_classes() -> Vec<String> {
    if !pinned_runtime() {
        return Vec::new();
    }
    let Ok(digest) = catalog_source_digest() else {
        return Vec::new();
    };
    if let Some(value) = IMMUTABLE_RUNTIME_CLASSES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&digest)
        .cloned()
    {
        return value;
    }
    let classes = [
        "java.lang.String",
        "java.lang.Boolean",
        "java.lang.Byte",
        "java.lang.Short",
        "java.lang.Integer",
        "java.lang.Long",
        "java.lang.Float",
        "java.lang.Double",
    ]
    .into_iter()
    .filter(|class| standard_immutable_type(class))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut cache = IMMUTABLE_RUNTIME_CLASSES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= 16 {
        cache.clear();
    }
    cache.insert(digest, classes.clone());
    classes
}
fn pinned_runtime() -> bool {
    crate::runtime::skript_version_is("2.15.4")
}
fn standard_immutable_type(class: &str) -> bool {
    let Ok(Some(option)) = type_for_class(class) else {
        return false;
    };
    if option.class_name != class
        || option.addon_name != "Skript"
        || option.addon_version != "2.15.4"
    {
        return false;
    }
    let Ok(digest) = catalog_source_digest() else {
        return false;
    };
    let key = format!("{digest}\0immutable\0{}\0{class}", option.registration_id);
    if let Some(value) = STANDARD_RUNTIME_SOURCES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .copied()
    {
        return value;
    }
    let result = registration_records(&option.registration_id)
        .ok()
        .is_some_and(|records| {
            let records = records
                .iter()
                .filter(|record| record.document == "Types.json")
                .collect::<Vec<_>>();
            records.len() == 1
                && serde_json::from_slice::<serde_json::Value>(&records[0].bytes)
                    .ok()
                    .is_some_and(|record| {
                        record["addon"]["name"].as_str() == Some("Skript")
                            && record["addon"]["version"].as_str() == Some("2.15.4")
                            && record["originalClass"].as_str() == Some(class)
                            && record["registrationId"].as_str()
                                == Some(option.registration_id.as_str())
                            && record["parserClass"].as_str().is_some_and(|value| {
                                value.starts_with("ch.njol.skript.classes.data.JavaClasses$")
                            })
                            && source_clone_identity(&record)
                    })
        });
    let mut cache = STANDARD_RUNTIME_SOURCES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_CACHED_CHANGE_CONTRACTS {
        cache.clear();
    }
    cache.insert(key, result);
    result
}
/// Schema 7's optional versioned dispatch facts come from the actual ClassInfo.
/// Missing facts (including every old snapshot) remain unknown. This is used
/// only after the pinned runtime, exact closed value class and source are bound.
pub(crate) fn source_clone_identity(record: &serde_json::Value) -> bool {
    const BASE: &str = "ch.njol.skript.classes.ClassInfo";
    let runtime = &record["typeRuntime"];
    runtime["version"].as_u64() == Some(1)
        && runtime["classInfoClass"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        && runtime["cloneMethodState"].as_str() == Some("known")
        && runtime["cloneMethodDeclaringClass"].as_str() == Some(BASE)
        && runtime["clonerGetterDeclaringClass"].as_str() == Some(BASE)
        && runtime["clonerState"].as_str() == Some("absent")
        && runtime["clonerClass"].is_null()
        && runtime["serializeAsGetterDeclaringClass"].as_str() == Some(BASE)
        && runtime["serializeAsState"].as_str() == Some("absent")
        && runtime["serializeAsClass"].is_null()
        && record["serializeAs"].is_null()
        && runtime["cloneMethodUnknownReason"].is_null()
        && runtime["clonerUnknownReason"].is_null()
        && runtime["serializeAsUnknownReason"].is_null()
}

/// The supported 2.15.4 PropertyBaseExpression implementations inherit the
/// default beforeChange. Bind that audit to the actual accepted registration;
/// a similarly named addon expression does not obtain this guarantee.
pub(crate) fn standard_property_source(
    payload: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionPayload,
) -> bool {
    let parameters = [
        "ch.njol.skript.lang.Expression".into(),
        "java.lang.Object[]".into(),
    ];
    standard_expression_source(payload, &payload.element_class)
        && [
            payload.element_class.as_str(),
            "org.skriptlang.skript.lang.properties.PropertyBaseExpression",
            "ch.njol.skript.lang.util.SimpleExpression",
        ]
        .into_iter()
        .all(|class| {
            matches!(
                declared_method_exists(class, "beforeChange", &parameters, None),
                Ok(Some(false))
            )
        })
        && registration_records(&payload.registration_id)
            .ok()
            .is_some_and(|records| {
                let records = records
                    .iter()
                    .filter(|record| record.document == "Expressions.json")
                    .collect::<Vec<_>>();
                records.len() == 1
                    && serde_json::from_slice::<serde_json::Value>(&records[0].bytes)
                        .ok()
                        .is_some_and(|record| {
                            record["superClass"].as_str()
                                == Some(
                                    "org.skriptlang.skript.lang.properties.PropertyBaseExpression",
                                )
                        })
            })
}

pub(crate) fn identity_delta_types_for_summary(
    summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary,
) -> Vec<String> {
    if !default_clone_input(summary) {
        return Vec::new();
    }
    identity_delta_classes()
}
fn default_clone_input(summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary) -> bool {
    // This discriminator is minted only by the native binder, and capture
    // summaries cannot be rewritten by registered hooks. A provider's Function
    // leaf does not receive it. The audited runtime ExprFunctionCall inherits
    // default beforeChange; identity still requires closed ClassInfo evidence.
    if summary.kind == "function" {
        let mut markers = summary
            .metadata
            .iter()
            .filter(|entry| entry.key == "parser.native-function-value-input");
        return pinned_runtime()
            && markers
                .next()
                .is_some_and(|entry| entry.owner_component_id.is_none() && entry.value == "1")
            && markers.next().is_none();
    }
    let entries = summary
        .metadata
        .iter()
        .filter(|entry| entry.key == crate::public_data::CHANGE_INPUT_METADATA_KEY)
        .collect::<Vec<_>>();
    entries.len() == 1
        && entries[0].owner_component_id.as_deref() == Some(crate::COMPONENT_ID)
        && clone_input_contract(&summary.kind, &entries[0].value).is_some()
}
// V2 certifies a closed runtime output domain independently of the advertised
// return bound and absence. Only the authenticated provider may mint it. Storage
// and cloning still require each actual ClassInfo's no-callback witness.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloneInput {
    version: u32,
    kind: String,
    #[serde(default)]
    runtime_classes: Vec<String>,
    #[serde(default)]
    classes_complete: bool,
}
fn clone_input_contract(kind: &str, json: &str) -> Option<CloneInput> {
    let input: CloneInput = serde_json::from_str(json).ok()?;
    if input.kind != "default-clone" {
        return None;
    }
    match input.version {
        1 if matches!(kind, "literal" | "variable")
            && input.runtime_classes.is_empty()
            && !input.classes_complete =>
        {
            Some(input)
        }
        2 if input.classes_complete
            && !input.runtime_classes.is_empty()
            && input.runtime_classes.len() <= 16
            && input.runtime_classes.iter().all(|class| !class.is_empty())
            && input
                .runtime_classes
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == input.runtime_classes.len() =>
        {
            Some(input)
        }
        _ => None,
    }
}
fn closed_clone_identity(
    summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary,
    identity_classes: &[String],
) -> bool {
    let mut entries = summary
        .metadata
        .iter()
        .filter(|entry| entry.key == crate::public_data::CHANGE_INPUT_METADATA_KEY);
    let Some(entry) = entries.next() else {
        return false;
    };
    if entries.next().is_some() || entry.owner_component_id.as_deref() != Some(crate::COMPONENT_ID)
    {
        return false;
    }
    clone_input_contract(&summary.kind, &entry.value).is_some_and(|input| {
        input.version == 2
            && input
                .runtime_classes
                .iter()
                .all(|class| identity_classes.contains(class))
    })
}
pub(crate) fn identity_delta_for_summary(
    summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary,
) -> bool {
    let classes = identity_delta_types_for_summary(summary);
    closed_clone_identity(summary, &classes) || summary.possible_return_types_state
        == crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete
        && !summary.possible_return_types.is_empty()
        && summary
            .return_type
            .as_ref()
            .is_some_and(|class| classes.contains(class))
        && summary
            .possible_return_types
            .iter()
            .all(|class| classes.contains(class))
}

pub(crate) fn can_convert(source_class: &str, target_class: &str) -> Result<TypeRelation, String> {
    can_convert_from_host(source_class, target_class)
}

pub(crate) fn is_class_assignable(
    source_class: &str,
    target_class: &str,
) -> Result<TypeRelation, String> {
    is_class_assignable_from_host(source_class, target_class)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn hierarchy_distance(superclass: &str, subclass: &str) -> Result<Option<u64>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::hierarchy_distance(superclass, subclass).map_err(|error| error.message)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn difference_options_for_type(
    input_class: &str,
) -> Result<Vec<DifferenceOption>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::difference_options_for_type(input_class)
        .map_err(|error| error.message)
        .map(|options| {
            options
                .into_iter()
                .map(|option| DifferenceOption {
                    input_class: option.input_class,
                    return_class: option.return_class,
                    registration_id: option.registration_id,
                    registration_order: option.registration_order,
                    hierarchy_distance: option.hierarchy_distance,
                })
                .collect()
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn difference_options_for_type(
    _input_class: &str,
) -> Result<Vec<DifferenceOption>, String> {
    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn hierarchy_distance(superclass: &str, subclass: &str) -> Result<Option<u64>, String> {
    Ok((superclass == subclass).then_some(0))
}

pub(crate) fn child_change_contract(
    child: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild,
) -> Result<Option<ChangeContract>, String> {
    let subject_id = child
        .registration_id
        .as_deref()
        .or(child.parser_id.as_deref());
    if let Some(subject_id) = subject_id
        && let Some(contract) = change_contract_from_metadata(&child.metadata, subject_id)?
    {
        return Ok(Some(contract));
    }
    if let Some(registration_id) = child.registration_id.as_deref()
        && let Some(contract) = expression_change_contract(registration_id)?
    {
        return Ok(Some(contract));
    }
    if child.kind == "expression-list" || child.kind == "variable" {
        return Ok(None);
    }
    child
        .return_type
        .as_deref()
        .map(type_change_contract)
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn container_element_type(class_name: &str) -> Result<Option<String>, String> {
    container_element_type_from_host(class_name)
}

pub(crate) fn class_known(class_name: &str) -> Result<bool, String> {
    class_known_from_host(class_name)
}

pub(crate) fn declared_method_exists(
    class_name: &str,
    method_name: &str,
    parameter_types: &[&str],
    return_type: Option<&str>,
) -> Result<Option<bool>, String> {
    declared_method_exists_from_host(class_name, method_name, parameter_types, return_type)
}

pub(crate) fn common_assignable_class(classes: &[String]) -> Result<Option<String>, String> {
    common_assignable_class_from_host(classes)
}

pub(crate) fn comparator_for_types(
    first_class: &str,
    second_class: &str,
) -> Result<ComparatorContract, String> {
    comparator_for_types_from_host(first_class, second_class)
}

pub(crate) fn property_handlers_for_type(
    property_name: &str,
    source_class: &str,
) -> Result<Vec<PropertyHandlerContract>, String> {
    property_handlers_for_type_from_host(property_name, source_class)
}

fn parse_change_contract(bytes: &[u8]) -> Result<ChangeContract, String> {
    let record = serde_json::from_slice::<ExpressionSourceRecord>(bytes)
        .map_err(|error| format!("invalid Expression source record: {error}"))?;
    Ok(match record.accepted_changers_state {
        ResolutionState::Resolved => ChangeContract::Resolved {
            modes: record
                .accepted_changers
                .unwrap_or_default()
                .into_iter()
                .map(|(mode, types)| {
                    (
                        mode,
                        types
                            .into_iter()
                            .map(|class_name| {
                                if let Some(element) = class_name.strip_suffix("[]") {
                                    AcceptedChangeType {
                                        class_name: element.to_owned(),
                                        multiple: true,
                                    }
                                } else {
                                    AcceptedChangeType {
                                        class_name,
                                        multiple: false,
                                    }
                                }
                            })
                            .collect(),
                    )
                })
                .collect(),
        },
        ResolutionState::Unresolved => ChangeContract::Unresolved,
    })
}

fn parse_event_value_change_contract(bytes: &[u8]) -> Result<ChangeContract, String> {
    let record = serde_json::from_slice::<EventValueSourceRecord>(bytes)
        .map_err(|error| format!("invalid EventValue source record: {error}"))?;
    Ok(match record.accepted_changers {
        Some(changers) => ChangeContract::Resolved {
            modes: changers
                .into_iter()
                .map(|(mode, types)| (mode, types.into_iter().map(accepted_change_type).collect()))
                .collect(),
        },
        None => ChangeContract::Unresolved,
    })
}

fn accepted_change_type(class_name: String) -> AcceptedChangeType {
    if let Some(element) = class_name.strip_suffix("[]") {
        AcceptedChangeType {
            class_name: element.to_owned(),
            multiple: true,
        }
    } else {
        AcceptedChangeType {
            class_name,
            multiple: false,
        }
    }
}

// Manifest work shares the ordinary guest fuel limit. Probe bounded record
// prefixes, then fetch only small records required by the provider's classifier.
// Each bound is independent: registrations outside these limits remain Unknown.
#[cfg(target_arch = "wasm32")]
pub(crate) fn expression_bound_sources(matches: fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    const MAX_RECORDS: u64 = 1_024;
    const PREFIX: u32 = 512;
    const MAX_BYTES: u32 = 64 * 1024;
    let source = catalog_data::source().ok()??;
    let mut bytes = vec![b'['];
    for index in 0..MAX_RECORDS {
        let Some(prefix) = catalog_data::read_record(
            &source.source_digest,
            &source.snapshot_id,
            "Expressions.json",
            index,
            0,
            PREFIX,
        )
        .ok()?
        else {
            break;
        };
        if prefix.offset != 0 || prefix.bytes.len() > PREFIX as usize {
            return None;
        }
        if prefix.total_length > MAX_BYTES as u64
            || bytes
                .len()
                .checked_add(usize::try_from(prefix.total_length).ok()?)?
                + 2
                > MAX_BYTES as usize
        {
            continue;
        }
        // Source records are normalized JSON objects. Probe both ends without
        // assuming field order; only a later full record can certify a bound.
        let hinted = if matches(&prefix.bytes) {
            true
        } else if prefix.total_length > PREFIX as u64 {
            let offset = prefix.total_length.saturating_sub(PREFIX as u64);
            let tail = catalog_data::read_record(
                &source.source_digest,
                &source.snapshot_id,
                "Expressions.json",
                index,
                offset,
                PREFIX,
            )
            .ok()??;
            if tail.offset != offset
                || tail.total_length != prefix.total_length
                || tail.bytes.len() > PREFIX as usize
            {
                return None;
            }
            matches(&tail.bytes)
        } else {
            false
        };
        if !hinted {
            continue;
        }
        let record = if prefix.bytes.len() as u64 == prefix.total_length {
            prefix
        } else {
            catalog_data::read_record(
                &source.source_digest,
                &source.snapshot_id,
                "Expressions.json",
                index,
                0,
                MAX_BYTES,
            )
            .ok()??
        };
        if record.offset != 0 || record.bytes.len() as u64 != record.total_length {
            return None;
        }
        if bytes.len() > 1 {
            bytes.push(b',');
        }
        bytes.extend_from_slice(&record.bytes);
    }
    bytes.push(b']');
    Some(bytes)
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn expression_bound_sources(_matches: fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    None
}

#[cfg(target_arch = "wasm32")]
fn registration_records(registration_id: &str) -> Result<Vec<SourceRecord>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    const PAGE_SIZE: u32 = 256;
    const CHUNK_SIZE: u32 = 1024 * 1024;

    let mut offset = 0;
    let mut records = Vec::new();
    let mut total_bytes = 0usize;
    loop {
        let page = catalog_data::records_by_registration_id(registration_id, offset, PAGE_SIZE)
            .map_err(|error| error.message)?;
        for record in page.items {
            if records.len() >= MAX_SOURCE_RECORDS_PER_LOOKUP {
                return Err(format!(
                    "registration ID {registration_id} exceeds the {MAX_SOURCE_RECORDS_PER_LOOKUP}-record lookup limit"
                ));
            }
            let record_bytes = usize::try_from(record.byte_length)
                .map_err(|_| "Catalog record length does not fit guest memory".to_owned())?;
            total_bytes = total_bytes.checked_add(record_bytes).ok_or_else(|| {
                "Catalog record lookup byte total overflowed guest memory".to_owned()
            })?;
            if total_bytes > MAX_SOURCE_BYTES_PER_LOOKUP {
                return Err(format!(
                    "registration ID {registration_id} exceeds the {MAX_SOURCE_BYTES_PER_LOOKUP}-byte lookup limit"
                ));
            }
            let mut bytes = Vec::with_capacity(record_bytes);
            let mut byte_offset = 0;
            loop {
                let chunk = catalog_data::read_record(
                    &record.source_digest,
                    &record.snapshot_id,
                    &record.document,
                    record.index,
                    byte_offset,
                    CHUNK_SIZE,
                )
                .map_err(|error| error.message)?
                .ok_or_else(|| {
                    format!(
                        "Catalog record {}[{}] disappeared while it was being read",
                        record.document, record.index
                    )
                })?;
                if chunk.offset != byte_offset || chunk.total_length != record.byte_length {
                    return Err(format!(
                        "Catalog record {}[{}] changed while it was being read",
                        record.document, record.index
                    ));
                }
                bytes.extend_from_slice(&chunk.bytes);
                byte_offset += chunk.bytes.len() as u64;
                if byte_offset >= chunk.total_length {
                    break;
                }
                if chunk.bytes.is_empty() {
                    return Err("Catalog returned an empty non-terminal record chunk".to_owned());
                }
            }
            records.push(SourceRecord {
                document: record.document,
                bytes,
            });
        }
        let Some(next_offset) = page.next_offset else {
            break;
        };
        if next_offset <= offset {
            return Err("Catalog record pagination did not advance".to_owned());
        }
        offset = next_offset;
    }
    Ok(records)
}

#[cfg(target_arch = "wasm32")]
fn catalog_source_digest() -> Result<String, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::source()
        .map_err(|error| error.message)?
        .map(|source| source.source_digest)
        .ok_or_else(|| "the host has no SSG source Catalog".to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn catalog_source_digest() -> Result<String, String> {
    Ok("native-tests".to_owned())
}

pub(crate) struct AliasMaterial {
    pub(crate) material: String,
    pub(crate) minecraft_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "non-collected states are supplied by the WIT host"
    )
)]
pub(crate) enum BlockDataRegistryState {
    Collected,
    Unsupported,
    Unresolved,
}

pub(crate) struct BlockDataStatus {
    pub(crate) state: BlockDataRegistryState,
    pub(crate) complete: bool,
    pub(crate) registry_provider: Option<String>,
    pub(crate) first_failure: Option<String>,
}

pub(crate) struct BlockDataEntry {
    pub(crate) default_state: String,
    pub(crate) properties: BTreeMap<String, Vec<String>>,
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn alias_materials(input: &str) -> Result<Option<Vec<AliasMaterial>>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::resolve_alias(input)
        .map_err(|error| error.message)
        .map(|target| {
            target.map(|target| {
                target
                    .types
                    .into_iter()
                    .map(|item| AliasMaterial {
                        material: item.material,
                        minecraft_id: item.minecraft_id,
                    })
                    .collect()
            })
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn alias_materials(_input: &str) -> Result<Option<Vec<AliasMaterial>>, String> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn block_data_status() -> Result<Option<BlockDataStatus>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::block_data_status()
        .map_err(|error| error.message)
        .map(|status| {
            status.map(|status| BlockDataStatus {
                state: match status.state {
                    catalog_data::CatalogBlockDataState::Collected => {
                        BlockDataRegistryState::Collected
                    }
                    catalog_data::CatalogBlockDataState::Unsupported => {
                        BlockDataRegistryState::Unsupported
                    }
                    catalog_data::CatalogBlockDataState::Unresolved => {
                        BlockDataRegistryState::Unresolved
                    }
                },
                complete: status.complete,
                registry_provider: status.registry_provider,
                first_failure: status.first_failure,
            })
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn block_data_status() -> Result<Option<BlockDataStatus>, String> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn block_data(id: &str) -> Result<Option<BlockDataEntry>, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::block_data(id)
        .map_err(|error| error.message)
        .map(|entry| {
            entry.map(|entry| BlockDataEntry {
                default_state: entry.default_state,
                properties: entry
                    .properties
                    .into_iter()
                    .map(|property| (property.name, property.values))
                    .collect(),
            })
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn block_data(_id: &str) -> Result<Option<BlockDataEntry>, String> {
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
fn registration_records(_registration_id: &str) -> Result<Vec<SourceRecord>, String> {
    Ok(Vec::new())
}

#[cfg(target_arch = "wasm32")]
fn can_convert_from_host(source_class: &str, target_class: &str) -> Result<TypeRelation, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    crate::nlaocs::skript_parser_addon::catalog_data::can_convert(source_class, target_class)
        .map(|relation| match relation {
            catalog_data::TypeRelation::Compatible => TypeRelation::Compatible,
            catalog_data::TypeRelation::Incompatible => TypeRelation::Incompatible,
            catalog_data::TypeRelation::Unknown => TypeRelation::Unknown,
        })
        .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn can_convert_from_host(source_class: &str, target_class: &str) -> Result<TypeRelation, String> {
    Ok(if source_class == target_class {
        TypeRelation::Compatible
    } else {
        TypeRelation::Unknown
    })
}

#[cfg(target_arch = "wasm32")]
fn is_class_assignable_from_host(
    source_class: &str,
    target_class: &str,
) -> Result<TypeRelation, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    crate::nlaocs::skript_parser_addon::catalog_data::is_class_assignable(
        source_class,
        target_class,
    )
    .map(|relation| match relation {
        catalog_data::TypeRelation::Compatible => TypeRelation::Compatible,
        catalog_data::TypeRelation::Incompatible => TypeRelation::Incompatible,
        catalog_data::TypeRelation::Unknown => TypeRelation::Unknown,
    })
    .map_err(|error| error.message)
}

#[cfg(target_arch = "wasm32")]
fn container_element_type_from_host(class_name: &str) -> Result<Option<String>, String> {
    crate::nlaocs::skript_parser_addon::catalog_data::container_element_type(class_name)
        .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn container_element_type_from_host(_class_name: &str) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
fn class_known_from_host(class_name: &str) -> Result<bool, String> {
    crate::nlaocs::skript_parser_addon::catalog_data::class_known(class_name)
        .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn class_known_from_host(_class_name: &str) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_arch = "wasm32")]
fn declared_method_exists_from_host(
    class_name: &str,
    method_name: &str,
    parameter_types: &[&str],
    return_type: Option<&str>,
) -> Result<Option<bool>, String> {
    let parameter_types = parameter_types
        .iter()
        .map(|parameter| (*parameter).to_owned())
        .collect::<Vec<_>>();
    crate::nlaocs::skript_parser_addon::catalog_data::declared_method_exists(
        class_name,
        method_name,
        &parameter_types,
        return_type,
    )
    .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn declared_method_exists_from_host(
    _class_name: &str,
    _method_name: &str,
    _parameter_types: &[&str],
    _return_type: Option<&str>,
) -> Result<Option<bool>, String> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
fn common_assignable_class_from_host(classes: &[String]) -> Result<Option<String>, String> {
    crate::nlaocs::skript_parser_addon::catalog_data::common_assignable_class(classes)
        .map_err(|error| error.message)
}

#[cfg(target_arch = "wasm32")]
fn comparator_for_types_from_host(
    first_class: &str,
    second_class: &str,
) -> Result<ComparatorContract, String> {
    use crate::nlaocs::skript_parser_addon::catalog_data;

    catalog_data::comparator_for_types(first_class, second_class)
        .map(|contract| ComparatorContract {
            relation: match contract.relation {
                catalog_data::TypeRelation::Compatible => TypeRelation::Compatible,
                catalog_data::TypeRelation::Incompatible => TypeRelation::Incompatible,
                catalog_data::TypeRelation::Unknown => TypeRelation::Unknown,
            },
            supports_ordering: contract.supports_ordering,
            supports_inversion: contract.supports_inversion,
            registration_id: contract.registration_id,
            reversed: contract.reversed,
        })
        .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn comparator_for_types_from_host(
    first_class: &str,
    second_class: &str,
) -> Result<ComparatorContract, String> {
    Ok(ComparatorContract {
        relation: if first_class == second_class {
            TypeRelation::Compatible
        } else {
            TypeRelation::Unknown
        },
        supports_ordering: (first_class == second_class).then_some(false),
        supports_inversion: (first_class == second_class).then_some(true),
        registration_id: None,
        reversed: false,
    })
}

#[cfg(target_arch = "wasm32")]
fn property_handlers_for_type_from_host(
    property_name: &str,
    source_class: &str,
) -> Result<Vec<PropertyHandlerContract>, String> {
    crate::nlaocs::skript_parser_addon::catalog_data::property_handlers_for_type(
        property_name,
        source_class,
    )
    .map(|handlers| {
        handlers
            .into_iter()
            .map(|handler| PropertyHandlerContract {
                property_registration_id: handler.property_registration_id,
                property_name: handler.property_name,
                input_class: handler.input_class,
                handler_class: handler.handler_class,
                handler_kind: handler.handler_kind,
                element_types: handler.element_types,
            })
            .collect()
    })
    .map_err(|error| error.message)
}

#[cfg(not(target_arch = "wasm32"))]
fn property_handlers_for_type_from_host(
    _property_name: &str,
    _source_class: &str,
) -> Result<Vec<PropertyHandlerContract>, String> {
    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
fn common_assignable_class_from_host(classes: &[String]) -> Result<Option<String>, String> {
    Ok(match classes {
        [] => None,
        [first, rest @ ..] if rest.iter().all(|class_name| class_name == first) => {
            Some(first.clone())
        }
        _ => Some("java.lang.Object".to_owned()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn is_class_assignable_from_host(
    source_class: &str,
    target_class: &str,
) -> Result<TypeRelation, String> {
    Ok(if source_class == target_class {
        TypeRelation::Compatible
    } else {
        TypeRelation::Unknown
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        AcceptedChangeMode, DynamicMultiplicity, RegisteredExpressionChild,
        RegisteredExpressionPropertyOption,
    };

    #[test]
    fn clone_input_requires_current_core_method_witness_not_just_value_class() {
        use crate::nlaocs::skript_parser_addon::types::{
            ExpressionPossibleReturnTypesState, ParseSummary,
        };
        let mut marker = crate::public_data::default_clone_change_input();
        marker.owner_component_id = Some(crate::COMPONENT_ID.into());
        let value = ParseSummary {
            kind: "literal".into(),
            definition_id: None,
            registration_id: None,
            element_class: None,
            pattern_index: None,
            return_type: Some("java.lang.String".into()),
            possible_return_types: vec!["java.lang.String".into()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: None,
            default_expression: None,
            public_data: Vec::new(),
            metadata: vec![marker],
        };
        assert!(default_clone_input(&value));
        let mut interpolation = value.clone();
        interpolation.public_data.push(
            crate::nlaocs::skript_parser_addon::types::ExpressionPublicData {
                schema_id: crate::public_data::INTERPOLATION_SCHEMA_ID.into(),
                schema_version: 1,
                json: "{}".into(),
            },
        );
        assert!(
            default_clone_input(&interpolation),
            "the explicit method witness applies after interpolation evaluation"
        );
        interpolation.kind = "variable".into();
        interpolation.metadata.clear();
        assert!(
            !default_clone_input(&interpolation),
            "unmarked dynamic names have no default method witness"
        );
        let mut changed = value.clone();
        changed.metadata[0].owner_component_id = Some("foreign".into());
        assert!(!default_clone_input(&changed));
        let mut changed = value.clone();
        changed.metadata[0].owner_component_id = None;
        assert!(!default_clone_input(&changed));
        let mut changed = value.clone();
        changed.metadata.push(changed.metadata[0].clone());
        assert!(!default_clone_input(&changed));
        let mut changed = value.clone();
        changed.kind = "registered-expression".into();
        assert!(!default_clone_input(&changed));
        let mut changed = value.clone();
        changed.metadata[0].value = r#"{"version":2,"kind":"default-clone"}"#.into();
        assert!(!default_clone_input(&changed));
    }

    #[test]
    fn closed_output_clone_contract_does_not_imply_presence_or_callback_purity() {
        let json = serde_json::json!({"version":2,"kind":"default-clone","runtimeClasses":["java.lang.Integer","java.lang.Double"],"classesComplete":true});
        assert!(clone_input_contract("registered-expression", &json.to_string()).is_some());
        for (field, value) in [
            ("classesComplete", serde_json::json!(false)),
            ("runtimeClasses", serde_json::json!([])),
            ("runtimeClasses", serde_json::json!(["x", "x"])),
            ("runtimeClasses", serde_json::json!([""])),
            ("nonEmpty", serde_json::json!(true)),
            ("version", serde_json::json!(3)),
        ] {
            let mut changed = json.clone();
            changed[field] = value;
            assert!(
                clone_input_contract("registered-expression", &changed.to_string()).is_none(),
                "{changed}"
            );
        }
    }

    #[test]
    fn genuine_type_runtime_snapshot_proves_selected_closed_class_dispatch() {
        let records: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-type-runtime/Types.json"
        )))
        .unwrap();
        for class in [
            "java.lang.String",
            "java.lang.Boolean",
            "java.lang.Byte",
            "java.lang.Short",
            "java.lang.Integer",
            "java.lang.Long",
            "java.lang.Float",
            "java.lang.Double",
        ] {
            let matching = records
                .iter()
                .filter(|record| record["originalClass"] == class)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "selected exact ClassInfo: {class}");
            let record = matching[0];
            assert_eq!(record["addon"]["name"], "Skript");
            assert_eq!(record["addon"]["version"], "2.15.4");
            assert!(source_clone_identity(record), "genuine dispatch: {class}");
        }
    }

    #[test]
    fn old_pinned_type_snapshot_keeps_clone_callback_behavior_unknown() {
        let records: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../parser-wasm/tests/data/type-parser-versions/skript-2.15.4/Types.json"
        )))
        .unwrap();
        for class in [
            "java.lang.String",
            "java.lang.Boolean",
            "java.lang.Byte",
            "java.lang.Short",
            "java.lang.Integer",
            "java.lang.Long",
            "java.lang.Float",
            "java.lang.Double",
        ] {
            let record = records
                .iter()
                .find(|record| record["originalClass"] == class)
                .expect(class);
            assert_eq!(record["addon"]["name"], "Skript");
            assert_eq!(record["addon"]["version"], "2.15.4");
            assert!(!source_clone_identity(record), "old fixture: {class}");
            assert!(
                record["parserClass"]
                    .as_str()
                    .unwrap()
                    .starts_with("ch.njol.skript.classes.data.JavaClasses$")
            );
        }
    }

    #[test]
    fn condition_local_read_requires_actual_pinned_source_class_and_pattern() {
        use crate::nlaocs::skript_parser_addon::types::{
            ConditionCandidate, MappedSpan, TextRange,
        };
        let records: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../parser-wasm/tests/data/type-parser-versions/skript-2.15.4/Conditions.json"
        )))
        .unwrap();
        let class = "ch.njol.skript.conditions.CondChance";
        let record = records
            .into_iter()
            .find(|record| record["elementClass"] == class)
            .unwrap();
        let candidate = ConditionCandidate {
            definition_id: record["definitionId"].as_str().unwrap().into(),
            registration_id: record["registrationId"].as_str().unwrap().into(),
            element_class: Some(class.into()),
            priority: 0,
            registration_order: 0,
            pattern_index: 0,
            pattern: record["patterns"][0].as_str().unwrap().into(),
            span: MappedSpan {
                virtual_range: TextRange { start: 0, end: 0 },
                origins: Vec::new(),
            },
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            marks: Vec::new(),
            handler: None,
            metadata: Vec::new(),
            children: Vec::new(),
            expression_bindings: Vec::new(),
        };
        assert!(standard_condition_source_record(
            &serde_json::to_vec(&record).unwrap(),
            &candidate,
            class
        ));
        for (field, replacement) in [
            (
                "addon",
                serde_json::json!({"name":"Addon","version":"2.15.4"}),
            ),
            (
                "addon",
                serde_json::json!({"name":"Skript","version":"2.16.0"}),
            ),
            ("registrationId", serde_json::json!("foreign")),
            ("definitionId", serde_json::json!("foreign")),
            ("elementClass", serde_json::json!("addon.CondChance")),
            ("kind", serde_json::json!("expression")),
            ("patterns", serde_json::json!(["different pattern"])),
        ] {
            let mut changed = record.clone();
            changed[field] = replacement;
            assert!(
                !standard_condition_source_record(
                    &serde_json::to_vec(&changed).unwrap(),
                    &candidate,
                    class
                ),
                "{field}"
            );
        }
        let mut changed = candidate.clone();
        changed.pattern_index = 99;
        assert!(!standard_condition_source_record(
            &serde_json::to_vec(&record).unwrap(),
            &changed,
            class
        ));
        assert!(
            !standard_condition_source(&candidate, class),
            "native tests have no authenticated Catalog source"
        );
    }

    #[test]
    fn clone_identity_requires_actual_base_dispatch_and_absent_callbacks() {
        let record = serde_json::json!({"typeRuntime": {
            "version":1,"classInfoClass":"ch.njol.skript.classes.ClassInfo",
            "cloneMethodState":"known",
            "cloneMethodDeclaringClass":"ch.njol.skript.classes.ClassInfo",
            "clonerGetterDeclaringClass":"ch.njol.skript.classes.ClassInfo",
            "clonerState":"absent",
            "serializeAsGetterDeclaringClass":"ch.njol.skript.classes.ClassInfo",
            "serializeAsState":"absent"
        }});
        assert!(source_clone_identity(&record));
        assert!(!source_clone_identity(&serde_json::json!({})));
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("classInfoClass", serde_json::Value::Null),
            ("cloneMethodState", serde_json::json!("unknown")),
            (
                "cloneMethodDeclaringClass",
                serde_json::json!("addon.Override"),
            ),
            (
                "clonerGetterDeclaringClass",
                serde_json::json!("addon.Override"),
            ),
            ("clonerState", serde_json::json!("present")),
            ("clonerClass", serde_json::json!("addon.Callback")),
            (
                "serializeAsGetterDeclaringClass",
                serde_json::json!("addon.Override"),
            ),
            ("serializeAsState", serde_json::json!("unknown")),
            ("serializeAsClass", serde_json::json!("java.lang.String")),
            (
                "clonerUnknownReason",
                serde_json::json!("overridden-accessor"),
            ),
        ] {
            let mut changed = record.clone();
            changed["typeRuntime"][key] = value;
            assert!(!source_clone_identity(&changed), "{key}");
        }
        let mut converted = record;
        converted["serializeAs"] = serde_json::json!("java.lang.String");
        assert!(!source_clone_identity(&converted));
    }

    #[test]
    fn event_value_local_read_without_getter_provenance_remains_unknown() {
        let value = EventValueOption {
            source_record: None,
            event_class: "org.bukkit.event.player.PlayerEvent".into(),
            value_class: "org.bukkit.entity.Player".into(),
            time: 0,
            registration_id: "event-value:skript:test".into(),
            patterns: Vec::new(),
            excludes: Vec::new(),
            exclude_error_message: None,
            resolution_order: 0,
            registration_order: Some(0),
            accepted_changers: BTreeMap::new(),
            context_dependent: Some(false),
            has_custom_input_validator: Some(false),
            has_custom_event_validator: Some(false),
        };
        let record = serde_json::json!({"addon":{"name":"Skript","version":"2.15.4"},
            "eventClass":value.event_class,"valueClass":value.value_class,"time":0,
            "registrationId":value.registration_id,"contextDependent":false,
            "hasCustomInputValidator":false,"hasCustomEventValidator":false});
        assert!(!standard_event_value_local_read(
            &serde_json::to_vec(&record).unwrap(),
            &value
        ));
        for (field, replacement) in [
            ("hasCustomInputValidator", serde_json::json!(true)),
            ("hasCustomEventValidator", serde_json::json!(null)),
            ("contextDependent", serde_json::json!(true)),
            ("registrationId", serde_json::json!("different")),
            (
                "addon",
                serde_json::json!({"name":"Addon","version":"2.15.4"}),
            ),
            (
                "addon",
                serde_json::json!({"name":"Skript","version":"2.16.0"}),
            ),
        ] {
            let mut changed = record.clone();
            changed[field] = replacement;
            assert!(
                !standard_event_value_local_read(&serde_json::to_vec(&changed).unwrap(), &value),
                "{field}"
            );
        }
        // Native tests have no authoritative source catalog. Even a matching
        // class cannot acquire a witness without the exact source record.
        assert!(!event_value_preserves_locals(&value, &value.value_class));
        assert!(!event_value_preserves_locals(&value, "java.lang.String"));
    }

    #[test]
    fn indexes_experiments_from_arbitrary_syntax_documents() {
        let value = serde_json::json!([{
            "definitionId": "effect:addon:test",
            "experimentalSyntax": {
                "required": [{
                    "codeName": "addon preview",
                    "phase": "experimental",
                    "known": true
                }],
                "disallowed": []
            }
        }]);
        let mut experiments = Vec::new();
        collect_experiments(&value, &mut experiments);
        assert_eq!(
            experiments,
            vec![CatalogExperiment {
                code_name: "addon preview".to_owned(),
                phase: "experimental".to_owned(),
                known: true,
            }]
        );
    }

    #[test]
    fn preserves_resolved_and_unresolved_change_contracts() {
        assert_eq!(
            parse_change_contract(
                br#"{"acceptedChangers":{"SET":["org.bukkit.Location"]},"acceptedChangersState":"resolved"}"#,
            )
            .unwrap(),
            ChangeContract::Resolved { modes: BTreeMap::from([(
                "SET".to_owned(),
                vec![AcceptedChangeType {
                    class_name: "org.bukkit.Location".to_owned(),
                    multiple: false,
                }],
            )]) }
        );
        assert_eq!(
            parse_change_contract(
                br#"{"acceptedChangers":{"SET":["java.lang.String[]"]},"acceptedChangersState":"resolved"}"#,
            )
            .unwrap(),
            ChangeContract::Resolved { modes: BTreeMap::from([(
                "SET".to_owned(),
                vec![AcceptedChangeType {
                    class_name: "java.lang.String".to_owned(),
                    multiple: true,
                }],
            )]) }
        );
        assert_eq!(
            parse_change_contract(br#"{"acceptedChangersState":"unresolved"}"#).unwrap(),
            ChangeContract::Unresolved
        );
        assert_eq!(
            parse_event_value_change_contract(
                br#"{"acceptedChangers":{"SET":["org.bukkit.Location"]}}"#
            )
            .unwrap(),
            ChangeContract::Resolved {
                modes: BTreeMap::from([(
                    "SET".to_owned(),
                    vec![AcceptedChangeType {
                        class_name: "org.bukkit.Location".to_owned(),
                        multiple: false,
                    }],
                )]),
            }
        );
        assert_eq!(
            parse_event_value_change_contract(br#"{"acceptedChangers":null}"#).unwrap(),
            ChangeContract::Unresolved
        );
    }

    #[test]
    fn metadata_contract_is_versioned_and_bound_to_its_expression() {
        let contract = ChangeContract::Resolved {
            modes: BTreeMap::new(),
        };
        let metadata = vec![change_contract_metadata("expression:test:0", &contract).unwrap()];

        assert_eq!(
            change_contract_from_metadata(&metadata, "expression:test:0").unwrap(),
            Some(contract)
        );
        assert!(
            change_contract_from_metadata(&metadata, "expression:other:0")
                .unwrap_err()
                .contains("attached")
        );
    }

    #[test]
    fn property_contracts_union_resolved_handler_types() {
        let options = [property_option(
            "org.bukkit.Location",
            MetadataResolutionState::Resolved,
            false,
            &["java.lang.Number", "java.lang.Number[]"],
        )];

        assert_eq!(
            property_change_contract(&options, None),
            ChangeContract::Resolved {
                modes: BTreeMap::from([(
                    "SET".to_owned(),
                    vec![
                        AcceptedChangeType {
                            class_name: "java.lang.Number".to_owned(),
                            multiple: false,
                        },
                        AcceptedChangeType {
                            class_name: "java.lang.Number".to_owned(),
                            multiple: true,
                        },
                    ],
                )]),
            }
        );
    }

    #[test]
    fn property_contract_requires_a_changeable_source_when_requested() {
        let options = [property_option(
            "org.bukkit.util.Vector",
            MetadataResolutionState::Resolved,
            true,
            &["java.lang.Number"],
        )];
        let changeable = source_child(ChangeContract::Resolved {
            modes: BTreeMap::from([(
                "SET".to_owned(),
                vec![AcceptedChangeType {
                    class_name: "org.bukkit.util.Vector".to_owned(),
                    multiple: false,
                }],
            )]),
        });
        let read_only = source_child(ChangeContract::Resolved {
            modes: BTreeMap::new(),
        });

        assert!(matches!(
            property_change_contract(&options, Some(&changeable)),
            ChangeContract::Resolved { modes } if modes.contains_key("SET")
        ));
        assert_eq!(
            property_change_contract(&options, Some(&read_only)),
            ChangeContract::Resolved {
                modes: BTreeMap::new()
            }
        );
        assert_eq!(
            property_change_contract(&options, None),
            ChangeContract::Unresolved
        );
    }

    #[test]
    fn property_contract_matches_skript_global_source_gate() {
        let options = [
            property_option(
                "org.bukkit.util.Vector",
                MetadataResolutionState::Resolved,
                true,
                &["java.lang.Float"],
            ),
            property_option(
                "org.bukkit.Location",
                MetadataResolutionState::Resolved,
                false,
                &["java.lang.String"],
            ),
        ];
        let source = source_child(ChangeContract::Resolved {
            modes: BTreeMap::from([(
                "SET".to_owned(),
                vec![AcceptedChangeType {
                    class_name: "org.bukkit.util.Vector".to_owned(),
                    multiple: false,
                }],
            )]),
        });

        assert_eq!(
            property_change_contract(&options, Some(&source)),
            ChangeContract::Resolved {
                modes: BTreeMap::from([(
                    "SET".to_owned(),
                    vec![
                        AcceptedChangeType {
                            class_name: "java.lang.Float".to_owned(),
                            multiple: false,
                        },
                        AcceptedChangeType {
                            class_name: "java.lang.String".to_owned(),
                            multiple: false,
                        },
                    ],
                )]),
            }
        );
    }

    #[test]
    fn unresolved_property_metadata_stays_unresolved() {
        let options = [property_option(
            "java.lang.Object",
            MetadataResolutionState::Unresolved,
            false,
            &[],
        )];
        assert_eq!(
            property_change_contract(&options, None),
            ChangeContract::Unresolved
        );
    }

    fn property_option(
        input_class: &str,
        state: MetadataResolutionState,
        requires_source: bool,
        accepted_types: &[&str],
    ) -> RegisteredExpressionPropertyOption {
        RegisteredExpressionPropertyOption {
            source_record: None,
            property_source_index: 0,
            related_type_index: 0,
            source_child_index: 0,
            match_kind: "exact".to_owned(),
            property_registration_id: "property:test".to_owned(),
            property_name: "test".to_owned(),
            property_handler_class: "test.PropertyHandler".to_owned(),
            property_addon_name: "TestAddon".to_owned(),
            property_addon_version: "1.0.0".to_owned(),
            input_class: input_class.to_owned(),
            handler_class: "test.TypePropertyHandler".to_owned(),
            handler_kind: "expression".to_owned(),
            provider_addon_name: Some("TestAddon".to_owned()),
            provider_addon_version: Some("1.0.0".to_owned()),
            type_code_name: "test".to_owned(),
            element_types: Vec::new(),
            return_types: vec!["java.lang.Object".to_owned()],
            supported_axes: Vec::new(),
            accepted_changers: vec![AcceptedChangeMode {
                mode: "SET".to_owned(),
                accepted_types: accepted_types
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            }],
            accepted_changers_state: Some(state),
            requires_source_expression_change: Some(requires_source),
        }
    }

    fn source_child(contract: ChangeContract) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            default_expression: None,
            text: "source".to_owned(),
            kind: "registered-expression".to_owned(),
            parser_id: None,
            definition_id: Some("expression:test:source".to_owned()),
            registration_id: Some("expression:test:source:0".to_owned()),
            pattern_index: Some(0),
            element_class: Some("test.Source".to_owned()),
            return_type: Some("org.bukkit.util.Vector".to_owned()),
            possible_return_types: vec!["org.bukkit.util.Vector".to_owned()],
            possible_return_types_state:
                crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            public_data: Vec::new(),
            metadata: vec![
                change_contract_metadata("expression:test:source:0", &contract).unwrap(),
            ],
        }
    }
}

/// Classes.toString searches ClassInfo in order before calling its parser.
/// Certify only the audited dense prefix: no custom getter or formatter can run
/// before these exact final runtime classes are selected. Old snapshots lack
/// the dispatch witness and deliberately certify no classes.
pub(crate) fn stringification_preserving_classes() -> Vec<String> {
    if !pinned_runtime() {
        return Vec::new();
    }
    let Ok(digest) = catalog_source_digest() else {
        return Vec::new();
    };
    let key = format!("{digest}\0stringification");
    if let Some(classes) = IMMUTABLE_RUNTIME_CLASSES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        return classes;
    }
    let classes = stringification_prefix(&stringification_records());
    let mut cache = IMMUTABLE_RUNTIME_CLASSES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= 16 {
        cache.clear();
    }
    cache.insert(key, classes.clone());
    classes
}
fn stringification_prefix(records: &[serde_json::Value]) -> Vec<String> {
    const BASE: &str = "ch.njol.skript.classes.ClassInfo";
    let audited = [
        ("Long", "LongParser"),
        ("Double", "DoubleParser"),
        ("Float", "FloatParser"),
        ("Boolean", "2"),
        ("Short", "ShortParser"),
        ("Byte", "ByteParser"),
        ("String", "5"),
    ];
    let mut classes = Vec::new();
    for (index, (class, parser)) in audited.into_iter().enumerate() {
        let Some(record) = records.get(index) else {
            break;
        };
        let class = format!("java.lang.{class}");
        let parser = format!("{}{}", "ch.njol.skript.classes.data.JavaClasses$", parser);
        if record["typeParseOrder"].as_u64() != Some(index as u64)
            || record["originalClass"].as_str() != Some(class.as_str())
            || record["addon"]["name"].as_str() != Some("Skript")
            || record["addon"]["version"].as_str() != Some("2.15.4")
            || record["hasParser"].as_bool() != Some(true)
            || record["parserClass"].as_str() != Some(parser.as_str())
            || record["typeRuntime"]["version"].as_u64() != Some(1)
            || record["typeRuntime"]["classInfoClass"].as_str() != Some(BASE)
        {
            break;
        }
        classes.push(class);
    }
    classes
}
#[cfg(target_arch = "wasm32")]
fn stringification_records() -> Vec<serde_json::Value> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    let Some(source) = catalog_data::source().ok().flatten() else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for index in 0..7 {
        let Some(chunk) = catalog_data::read_record(
            &source.source_digest,
            &source.snapshot_id,
            "Types.json",
            index,
            0,
            64 * 1024,
        )
        .ok()
        .flatten() else {
            break;
        };
        if chunk.offset != 0 || chunk.bytes.len() as u64 != chunk.total_length {
            break;
        }
        let Ok(record) = serde_json::from_slice(&chunk.bytes) else {
            break;
        };
        records.push(record);
    }
    records
}
#[cfg(not(target_arch = "wasm32"))]
fn stringification_records() -> Vec<serde_json::Value> {
    Vec::new()
}

/// Exact registered type lookup uses the base ClassInfo's non-callback getters.
pub(crate) fn base_type_dispatch(class: &str) -> bool {
    if !pinned_runtime() {
        return false;
    }
    let Ok(Some(option)) = type_for_class(class) else {
        return false;
    };
    if option.class_name != class {
        return false;
    }
    let Ok(digest) = catalog_source_digest() else {
        return false;
    };
    let key = format!("{digest}\0base-dispatch\0{}", option.registration_id);
    if let Some(value) = STANDARD_RUNTIME_SOURCES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .copied()
    {
        return value;
    }
    let result = registration_records(&option.registration_id)
        .ok()
        .is_some_and(|records| {
            let records = records
                .iter()
                .filter(|record| record.document == "Types.json")
                .collect::<Vec<_>>();
            records.len() == 1
                && serde_json::from_slice::<serde_json::Value>(&records[0].bytes)
                    .ok()
                    .is_some_and(|record| {
                        record["originalClass"].as_str() == Some(class)
                            && record["registrationId"].as_str()
                                == Some(option.registration_id.as_str())
                            && record["typeRuntime"]["version"].as_u64() == Some(1)
                            && record["typeRuntime"]["classInfoClass"].as_str()
                                == Some("ch.njol.skript.classes.ClassInfo")
                    })
        });
    let mut cache = STANDARD_RUNTIME_SOURCES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_CACHED_CHANGE_CONTRACTS {
        cache.clear();
    }
    cache.insert(key, result);
    result
}

#[cfg(test)]
mod evaluation_context_tests {
    use super::*;
    #[test]
    fn formatter_prefix_needs_actual_order_and_base_dispatch() {
        let mut records: Vec<serde_json::Value> = serde_json::from_str(include_str!("../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-type-runtime/Types.json")).unwrap();
        assert_eq!(stringification_prefix(&records).len(), 7);
        for field in ["typeRuntime", "parserClass", "typeParseOrder", "addon"] {
            let mut changed = records.clone();
            changed[0].as_object_mut().unwrap().remove(field);
            assert!(stringification_prefix(&changed).is_empty(), "{field}");
        }
        records[1]["typeRuntime"]["classInfoClass"] = serde_json::json!("addon.Override");
        assert_eq!(stringification_prefix(&records), ["java.lang.Long"]);
        let old: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4/Types.json"
        ))
        .unwrap();
        assert!(stringification_prefix(&old).is_empty());
    }
}

/// Distinguish no changer from a changer that rejects every mode.
pub(crate) fn type_init_has_no_changer(class: &str) -> bool {
    if !base_type_dispatch(class) {
        return false;
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::nlaocs::skript_parser_addon::catalog_data::change_contract_for_type(class)
            .ok()
            .flatten()
            .is_some_and(|contract| contract.type_class == class && !contract.has_changer)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// A primitive value result does not certify the parser that created it. Bind
/// the actual active registration to its audited parse-time implementation too.
pub(crate) fn standard_primitive_parser(
    active: &crate::nlaocs::skript_parser_addon::types::ExpressionTypeOption,
) -> bool {
    let suffix = match active.class_name.as_str() {
        "java.lang.String" => "5",
        "java.lang.Boolean" => "2",
        "java.lang.Number" => "NumberParser",
        "java.lang.Long" => "LongParser",
        "java.lang.Double" => "DoubleParser",
        "java.lang.Float" => "FloatParser",
        "java.lang.Short" => "ShortParser",
        "java.lang.Byte" => "ByteParser",
        "java.lang.Integer" => "IntegerParser",
        _ => return false,
    };
    let expected = format!("{}{}", "ch.njol.skript.classes.data.JavaClasses$", suffix);
    active.parser_class.as_deref() == Some(expected.as_str())
        && type_for_class(&active.class_name)
            .ok()
            .flatten()
            .is_some_and(|first| {
                first.registration_id == active.registration_id
                    && first.parser_class == active.parser_class
            })
        && base_type_dispatch(&active.class_name)
}
