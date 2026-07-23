use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use super::{
    Address, NamespaceKey, NamespaceRecord, StateEncoding, StateError, StateValue, apply_overlay,
    persistence_error,
};

const VALUES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("state_values");
const NAMESPACES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("state_namespaces");

pub(super) struct PersistentProject {
    database: Database,
    pub(super) values: BTreeMap<NamespaceKey, BTreeMap<String, StateValue>>,
    pub(super) revisions: BTreeMap<NamespaceKey, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NamespaceMetadata {
    schema_id: String,
    schema_version: u32,
    revision: u64,
}

impl PersistentProject {
    pub(super) fn open(
        path: &Path,
        records: &BTreeMap<NamespaceKey, NamespaceRecord>,
    ) -> Result<Self, StateError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(persistence_error)?;
        }
        let database = Database::create(path).map_err(persistence_error)?;
        let mut project = Self {
            database,
            values: BTreeMap::new(),
            revisions: BTreeMap::new(),
        };
        project.synchronize(records)?;
        Ok(project)
    }

    pub(super) fn synchronize(
        &mut self,
        records: &BTreeMap<NamespaceKey, NamespaceRecord>,
    ) -> Result<(), StateError> {
        let stored = self.read_metadata()?;
        let mut metadata = BTreeMap::new();
        let mut reset = BTreeSet::new();
        for (namespace, record) in records {
            let declaration = &record.declaration;
            let previous = stored.get(namespace);
            let schema_changed = previous.is_some_and(|previous| {
                previous.schema_id != declaration.schema_id
                    || previous.schema_version != declaration.schema_version
            });
            let revision = previous.map_or(0, |previous| {
                if schema_changed {
                    previous.revision.saturating_add(1)
                } else {
                    previous.revision
                }
            });
            if schema_changed {
                reset.insert(namespace.clone());
            }
            metadata.insert(
                namespace.clone(),
                NamespaceMetadata {
                    schema_id: declaration.schema_id.clone(),
                    schema_version: declaration.schema_version,
                    revision,
                },
            );
        }

        let write = self.database.begin_write().map_err(persistence_error)?;
        {
            let mut values = write.open_table(VALUES).map_err(persistence_error)?;
            if !reset.is_empty() {
                let mut keys = Vec::new();
                for entry in values.iter().map_err(persistence_error)? {
                    let (key, _) = entry.map_err(persistence_error)?;
                    let bytes = key.value();
                    if decode_persistent_key(bytes)
                        .is_ok_and(|(namespace, _)| reset.contains(&namespace))
                    {
                        keys.push(bytes.to_vec());
                    }
                }
                for key in keys {
                    values.remove(key.as_slice()).map_err(persistence_error)?;
                }
            }
        }
        {
            let mut namespaces = write.open_table(NAMESPACES).map_err(persistence_error)?;
            for (namespace, value) in &metadata {
                let key = encode_namespace(namespace);
                let value = encode_metadata(value);
                namespaces
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(persistence_error)?;
            }
        }
        write.commit().map_err(persistence_error)?;
        self.reload(records)
    }

    pub(super) fn commit(
        &mut self,
        writes: &BTreeMap<Address, Option<StateValue>>,
        records: &BTreeMap<NamespaceKey, NamespaceRecord>,
    ) -> Result<(), StateError> {
        let mut touched = BTreeSet::new();
        let write = self.database.begin_write().map_err(persistence_error)?;
        {
            let mut values = write.open_table(VALUES).map_err(persistence_error)?;
            for (address, replacement) in writes {
                let key = encode_persistent_key(&address.namespace, &address.key);
                match replacement {
                    Some(value) => {
                        let value = encode_value(value);
                        values
                            .insert(key.as_slice(), value.as_slice())
                            .map_err(persistence_error)?;
                    }
                    None => {
                        values.remove(key.as_slice()).map_err(persistence_error)?;
                    }
                }
                touched.insert(address.namespace.clone());
            }
        }
        {
            let mut namespaces = write.open_table(NAMESPACES).map_err(persistence_error)?;
            for namespace in &touched {
                let record = records.get(namespace).ok_or_else(|| StateError::Internal {
                    message: format!(
                        "persistent namespace {} disappeared during commit",
                        namespace.name()
                    ),
                })?;
                let revision = self
                    .revisions
                    .get(namespace)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
                let metadata = NamespaceMetadata {
                    schema_id: record.declaration.schema_id.clone(),
                    schema_version: record.declaration.schema_version,
                    revision,
                };
                let key = encode_namespace(namespace);
                let value = encode_metadata(&metadata);
                namespaces
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(persistence_error)?;
            }
        }
        write.commit().map_err(persistence_error)?;

        for (address, replacement) in writes {
            let namespace = self.values.entry(address.namespace.clone()).or_default();
            apply_overlay(namespace, &address.key, replacement);
        }
        for namespace in touched {
            let revision = self.revisions.entry(namespace).or_default();
            *revision = revision.saturating_add(1);
        }
        Ok(())
    }

    fn read_metadata(&self) -> Result<BTreeMap<NamespaceKey, NamespaceMetadata>, StateError> {
        let read = self.database.begin_read().map_err(persistence_error)?;
        let table = match read.open_table(NAMESPACES) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(error) => return Err(persistence_error(error)),
        };
        let mut metadata = BTreeMap::new();
        for entry in table.iter().map_err(persistence_error)? {
            let (key, value) = entry.map_err(persistence_error)?;
            let namespace = decode_namespace_exact(key.value())?;
            metadata.insert(namespace, decode_metadata(value.value())?);
        }
        Ok(metadata)
    }

    fn reload(
        &mut self,
        records: &BTreeMap<NamespaceKey, NamespaceRecord>,
    ) -> Result<(), StateError> {
        self.values.clear();
        self.revisions.clear();
        let metadata = self.read_metadata()?;
        for (namespace, record) in records {
            let Some(stored) = metadata.get(namespace) else {
                continue;
            };
            if stored.schema_id == record.declaration.schema_id
                && stored.schema_version == record.declaration.schema_version
            {
                self.revisions.insert(namespace.clone(), stored.revision);
            }
        }

        let read = self.database.begin_read().map_err(persistence_error)?;
        let table = match read.open_table(VALUES) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(()),
            Err(error) => return Err(persistence_error(error)),
        };
        for entry in table.iter().map_err(persistence_error)? {
            let (key, value) = entry.map_err(persistence_error)?;
            let (namespace, key) = decode_persistent_key(key.value())?;
            let Some(record) = records.get(&namespace) else {
                continue;
            };
            let value = decode_value(value.value())?;
            if value.schema_id != record.declaration.schema_id {
                continue;
            }
            self.values.entry(namespace).or_default().insert(key, value);
        }
        Ok(())
    }
}

fn encode_namespace(namespace: &NamespaceKey) -> Vec<u8> {
    let mut output = Vec::new();
    match namespace {
        NamespaceKey::Private { owner, name } => {
            output.push(0);
            push_string(&mut output, owner);
            push_string(&mut output, name);
        }
        NamespaceKey::Shared { name } => {
            output.push(1);
            push_string(&mut output, name);
        }
    }
    output
}

fn decode_namespace(bytes: &[u8]) -> Result<(NamespaceKey, usize), StateError> {
    let (&tag, rest) = bytes
        .split_first()
        .ok_or_else(|| malformed("empty namespace key"))?;
    let mut offset = 1;
    match tag {
        0 => {
            let (owner, used) = read_string(rest)?;
            offset += used;
            let (name, used) = read_string(&bytes[offset..])?;
            offset += used;
            Ok((NamespaceKey::Private { owner, name }, offset))
        }
        1 => {
            let (name, used) = read_string(rest)?;
            offset += used;
            Ok((NamespaceKey::Shared { name }, offset))
        }
        _ => Err(malformed("unknown namespace visibility tag")),
    }
}

fn decode_namespace_exact(bytes: &[u8]) -> Result<NamespaceKey, StateError> {
    let (namespace, used) = decode_namespace(bytes)?;
    if used != bytes.len() {
        return Err(malformed("namespace key has trailing bytes"));
    }
    Ok(namespace)
}

fn encode_persistent_key(namespace: &NamespaceKey, key: &str) -> Vec<u8> {
    let mut output = encode_namespace(namespace);
    push_string(&mut output, key);
    output
}

fn decode_persistent_key(bytes: &[u8]) -> Result<(NamespaceKey, String), StateError> {
    let (namespace, used) = decode_namespace(bytes)?;
    let (key, key_used) = read_string(&bytes[used..])?;
    if used + key_used != bytes.len() {
        return Err(malformed("persistent key has trailing bytes"));
    }
    Ok((namespace, key))
}

fn encode_value(value: &StateValue) -> Vec<u8> {
    let mut output = Vec::new();
    push_string(&mut output, &value.schema_id);
    output.push(match value.encoding {
        StateEncoding::Raw => 0,
        StateEncoding::Cbor => 1,
        StateEncoding::Json => 2,
    });
    output.extend_from_slice(&value.bytes);
    output
}

fn decode_value(bytes: &[u8]) -> Result<StateValue, StateError> {
    let (schema_id, used) = read_string(bytes)?;
    let (&encoding, payload) = bytes[used..]
        .split_first()
        .ok_or_else(|| malformed("state value has no encoding"))?;
    let encoding = match encoding {
        0 => StateEncoding::Raw,
        1 => StateEncoding::Cbor,
        2 => StateEncoding::Json,
        _ => return Err(malformed("state value has an unknown encoding")),
    };
    Ok(StateValue::new(schema_id, encoding, payload))
}

fn encode_metadata(metadata: &NamespaceMetadata) -> Vec<u8> {
    let mut output = Vec::new();
    push_string(&mut output, &metadata.schema_id);
    output.extend_from_slice(&metadata.schema_version.to_le_bytes());
    output.extend_from_slice(&metadata.revision.to_le_bytes());
    output
}

fn decode_metadata(bytes: &[u8]) -> Result<NamespaceMetadata, StateError> {
    let (schema_id, used) = read_string(bytes)?;
    if bytes.len() != used + 12 {
        return Err(malformed("namespace metadata has an invalid length"));
    }
    let schema_version = u32::from_le_bytes(
        bytes[used..used + 4]
            .try_into()
            .expect("slice length was checked"),
    );
    let revision = u64::from_le_bytes(
        bytes[used + 4..]
            .try_into()
            .expect("slice length was checked"),
    );
    Ok(NamespaceMetadata {
        schema_id,
        schema_version,
        revision,
    })
}

fn push_string(output: &mut Vec<u8>, value: &str) {
    let length = u32::try_from(value.len()).expect("StateStore strings are quota limited");
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn read_string(bytes: &[u8]) -> Result<(String, usize), StateError> {
    let length_bytes: [u8; 4] = bytes
        .get(..4)
        .ok_or_else(|| malformed("length-prefixed string is truncated"))?
        .try_into()
        .expect("slice length was checked");
    let length = u32::from_le_bytes(length_bytes) as usize;
    let end = 4usize
        .checked_add(length)
        .ok_or_else(|| malformed("length-prefixed string overflows"))?;
    let value = bytes
        .get(4..end)
        .ok_or_else(|| malformed("length-prefixed string is truncated"))?;
    let value =
        std::str::from_utf8(value).map_err(|_| malformed("length-prefixed string is not UTF-8"))?;
    Ok((value.to_owned(), end))
}

fn malformed(message: &str) -> StateError {
    StateError::Persistence {
        message: format!("malformed persistent StateStore data: {message}"),
    }
}
