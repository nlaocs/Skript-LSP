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
const MAX_SOURCE_RECORDS_PER_LOOKUP: usize = 256;
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

struct SourceRecord {
    document: String,
    bytes: Vec<u8>,
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

pub(crate) fn can_convert(source_class: &str, target_class: &str) -> Result<TypeRelation, String> {
    can_convert_from_host(source_class, target_class)
}

pub(crate) fn is_class_assignable(
    source_class: &str,
    target_class: &str,
) -> Result<TypeRelation, String> {
    is_class_assignable_from_host(source_class, target_class)
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
            text: "source".to_owned(),
            kind: "registered-expression".to_owned(),
            parser_id: None,
            definition_id: Some("expression:test:source".to_owned()),
            registration_id: Some("expression:test:source:0".to_owned()),
            pattern_index: Some(0),
            element_class: Some("test.Source".to_owned()),
            return_type: Some("org.bukkit.util.Vector".to_owned()),
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![
                change_contract_metadata("expression:test:source:0", &contract).unwrap(),
            ],
        }
    }
}
