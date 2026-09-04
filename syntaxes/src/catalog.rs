//! Immutable indexes and semantic queries over normalized syntax data.
//!
//! Indexes preserve generator registration order and provide cycle-safe traversal
//! for Java assignability, converters, event values, functions, and aliases.
#![allow(missing_docs)] // Public fields are described by their owning domain type.

use crate::{
    AliasRegistry, Class, ClassKind, ClassName, Comparator, Converter, Difference, EventValue,
    Function, Operation, Operator, Property, Syntax, Type, TypeLiteral,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::Arc,
};
use syntax_pattern_parser::syntax::PluralRules;

#[derive(Debug, Clone, PartialEq, Eq)]
/// One top-level JSON object retained from the source snapshot.
pub struct CatalogSourceRecord {
    pub document: String,
    pub index: usize,
    pub json: Arc<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Runtime server and plugin identity retained from SSG Manifest.json.
pub struct CatalogRuntime {
    pub server_name: String,
    pub server_version: String,
    pub minecraft_version: String,
    pub java_version: String,
    pub language: String,
    pub plugins: Vec<CatalogRuntimePlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One runtime plugin in captured server load order.
pub struct CatalogRuntimePlugin {
    pub load_order: usize,
    pub name: String,
    pub version: String,
    pub main: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
/// Opaque, forward-compatible documents and indexes behind a normalized catalog.
///
/// The parser uses normalized [`Catalog`] values. WASM addons and diagnostics can
/// use this source view when they need an SSG field that the normalized model does
/// not interpret yet. Original document bytes retain unknown fields verbatim;
/// indexed records are re-serialized JSON objects and do not promise whitespace or
/// object-key order.
pub struct CatalogSource {
    pub format: String,
    pub schema_version: u32,
    pub snapshot_id: String,
    /// Digest of every retained source filename and exact byte sequence.
    pub source_digest: String,
    pub runtime: Option<CatalogRuntime>,
    documents: BTreeMap<String, Arc<[u8]>>,
    records: HashMap<(String, usize), CatalogSourceRecord>,
    registration_records: HashMap<String, Vec<CatalogSourceRecord>>,
    definition_records: HashMap<String, Vec<CatalogSourceRecord>>,
}

impl CatalogSource {
    /// Retains caller-validated source documents and indexes top-level registration objects.
    ///
    /// This type does not verify snapshot digests itself. Production callers should obtain it
    /// through `ssg::load`; direct construction is intended for other validated producers and tests.
    pub fn from_json_documents(
        format: impl Into<String>,
        schema_version: u32,
        snapshot_id: impl Into<String>,
        documents: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, serde_json::Error> {
        let documents = documents
            .into_iter()
            .map(|(name, bytes)| (name, Arc::<[u8]>::from(bytes)))
            .collect::<BTreeMap<_, _>>();
        let mut registration_records: HashMap<String, Vec<CatalogSourceRecord>> = HashMap::new();
        let mut definition_records: HashMap<String, Vec<CatalogSourceRecord>> = HashMap::new();
        let mut indexed_records = HashMap::new();

        for (document, bytes) in &documents {
            let value: Value = serde_json::from_slice(bytes)?;
            let Some(records) = value.as_array() else {
                continue;
            };
            for (index, value) in records.iter().enumerate() {
                let Some(object) = value.as_object() else {
                    continue;
                };
                let record = CatalogSourceRecord {
                    document: document.clone(),
                    index,
                    json: Arc::from(serde_json::to_vec(value)?),
                };
                indexed_records.insert((document.clone(), index), record.clone());
                if let Some(id) = object.get("registrationId").and_then(Value::as_str) {
                    registration_records
                        .entry(id.to_owned())
                        .or_default()
                        .push(record.clone());
                }
                if let Some(id) = object.get("definitionId").and_then(Value::as_str) {
                    definition_records
                        .entry(id.to_owned())
                        .or_default()
                        .push(record);
                }
            }
        }

        let source_digest = source_digest(&documents);
        Ok(Self {
            format: format.into(),
            schema_version,
            snapshot_id: snapshot_id.into(),
            source_digest,
            runtime: None,
            documents,
            records: indexed_records,
            registration_records,
            definition_records,
        })
    }

    /// Attaches runtime identity validated alongside the retained documents.
    pub fn with_runtime(mut self, runtime: CatalogRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Returns source document names in deterministic order.
    pub fn document_names(&self) -> impl Iterator<Item = &str> {
        self.documents.keys().map(String::as_str)
    }

    /// Returns the exact caller-retained bytes of one source document.
    pub fn document(&self, name: &str) -> Option<&[u8]> {
        self.documents.get(name).map(AsRef::as_ref)
    }

    /// Returns one indexed top-level JSON object by its source location.
    pub fn record(&self, document: &str, index: usize) -> Option<&CatalogSourceRecord> {
        self.records.get(&(document.to_owned(), index))
    }

    /// Returns every raw object carrying the requested registration ID.
    pub fn records_by_registration_id(&self, id: &str) -> &[CatalogSourceRecord] {
        self.registration_records.get(id).map_or(&[], Vec::as_slice)
    }

    /// Returns every raw object carrying the requested definition ID.
    pub fn records_by_definition_id(&self, id: &str) -> &[CatalogSourceRecord] {
        self.definition_records.get(id).map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Clone)]
/// Explicit, unindexed inputs used to construct a `Catalog`.
pub struct CatalogParts {
    pub syntaxes: Vec<Syntax>,
    pub converters: Vec<Converter>,
    pub comparators: Vec<Comparator>,
    pub event_values: Vec<EventValue>,
    pub properties: Vec<Property>,
    pub operators: Vec<Operator>,
    pub operations: BTreeMap<String, Vec<Operation>>,
    pub differences: Vec<Difference>,
    pub classes: Vec<Class>,
    pub aliases: AliasRegistry,
    pub plural_rules: PluralRules,
    /// Effective global language entries collected from the SSG snapshot.
    pub language: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
/// Immutable normalized registry with indexes for parser and LSP queries.
///
/// When built by `ssg`, a catalog is the semantic view of one complete server
/// snapshot. `CatalogParts` also permits partial catalogs for tests and other
/// validated producers. It keeps generator registration order while adding
/// indexes for syntax IDs, type code names, functions, converters, event values,
/// aliases, and Java class
/// relationships. Parser code should share a single catalog rather than copy
/// individual JSON arrays.
///
/// # Examples
///
/// Functions can accept a catalog without depending on the SSG file format:
///
/// ~~~
/// use syntaxes::{Catalog, SyntaxKind};
///
/// fn summarize(catalog: &Catalog) -> (usize, usize, bool) {
///     let effect_count = catalog
///         .syntaxes()
///         .iter()
///         .filter(|syntax| syntax.kind() == SyntaxKind::Effect)
///         .count();
///     let function_count = catalog.functions().count();
///     let strings_are_objects = catalog
///         .type_by_code_name("string")
///         .is_some_and(|ty| {
///             catalog.is_class_assignable(
///                 ty.original_class.as_str(),
///                 "java.lang.Object",
///             )
///         });
///
///     (effect_count, function_count, strings_are_objects)
/// }
/// # let _ = summarize;
/// ~~~
pub struct Catalog {
    parts: CatalogParts,
    index: CatalogIndex,
    source: Option<Arc<CatalogSource>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The source of a finite type literal accepted by the catalog.
pub enum TypeLiteralSource {
    ParserPattern,
    Supplier,
    EnumConstant,
    Alias,
}

#[derive(Debug, Clone, PartialEq)]
/// Semantic information about one exact type-literal match.
pub struct TypeLiteralMatch<'a> {
    pub type_info: &'a Type,
    pub literal: Option<&'a TypeLiteral>,
    pub literal_index: Option<usize>,
    pub canonical_value: &'a str,
    pub plural: bool,
    pub source: TypeLiteralSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypeLiteralIndexEntry {
    type_position: usize,
    literal_position: Option<usize>,
    canonical_value: String,
    plural: bool,
    source: TypeLiteralSource,
}

#[derive(Debug, Clone, Default)]
struct CatalogIndex {
    syntaxes_by_registration_id: HashMap<String, Vec<usize>>,
    types_by_code_name: HashMap<String, usize>,
    functions_by_name: HashMap<String, Vec<usize>>,
    event_values_by_event_class: HashMap<String, Vec<usize>>,
    converters_by_from: HashMap<String, Vec<usize>>,
    converters_by_to: HashMap<String, Vec<usize>>,
    classes_by_name: HashMap<String, usize>,
    type_literals: HashMap<String, Vec<TypeLiteralIndexEntry>>,
}

fn index_type_literals(
    index: &mut HashMap<String, Vec<TypeLiteralIndexEntry>>,
    type_position: usize,
    literals: &[String],
    source: TypeLiteralSource,
    pluralize: bool,
    plural_rules: &PluralRules,
) {
    for literal in literals {
        index_type_literal(index, type_position, None, literal, literal, false, source);
        if pluralize {
            let plural = plural_rules.to_plural(literal);
            if plural != *literal {
                index_type_literal(index, type_position, None, &plural, literal, true, source);
            }
        }
    }
}

fn index_type_literal(
    index: &mut HashMap<String, Vec<TypeLiteralIndexEntry>>,
    type_position: usize,
    literal_position: Option<usize>,
    literal: &str,
    canonical_value: &str,
    plural: bool,
    source: TypeLiteralSource,
) {
    let normalized = normalize_literal(literal);
    if normalized.is_empty() {
        return;
    }
    index
        .entry(normalized)
        .or_default()
        .push(TypeLiteralIndexEntry {
            type_position,
            literal_position,
            canonical_value: canonical_value.to_owned(),
            plural,
            source,
        });
}

impl Catalog {
    /// Builds all deterministic indexes from validated normalized parts.
    pub fn new(parts: CatalogParts) -> Self {
        let mut index = CatalogIndex::default();

        for (position, syntax) in parts.syntaxes.iter().enumerate() {
            index
                .syntaxes_by_registration_id
                .entry(syntax.registration_id().as_str().to_owned())
                .or_default()
                .push(position);

            match syntax {
                Syntax::Type(value) => {
                    index
                        .types_by_code_name
                        .insert(value.code_name.as_str().to_owned(), position);
                    if value.has_parser {
                        index_type_literals(
                            &mut index.type_literals,
                            position,
                            &value.parser_patterns,
                            TypeLiteralSource::ParserPattern,
                            false,
                            &parts.plural_rules,
                        );
                        if value.type_literals.is_empty() {
                            // Schema 3 snapshots created before structured supplier values
                            // still use the exported PluralRules as a compatibility fallback.
                            index_type_literals(
                                &mut index.type_literals,
                                position,
                                &value.literal_values,
                                TypeLiteralSource::Supplier,
                                true,
                                &parts.plural_rules,
                            );
                        } else {
                            for (literal_position, literal) in
                                value.type_literals.iter().enumerate()
                            {
                                index_type_literal(
                                    &mut index.type_literals,
                                    position,
                                    Some(literal_position),
                                    &literal.text,
                                    &literal.text,
                                    false,
                                    TypeLiteralSource::Supplier,
                                );
                                if let Some(plural) = literal.plural_text.as_deref() {
                                    index_type_literal(
                                        &mut index.type_literals,
                                        position,
                                        Some(literal_position),
                                        plural,
                                        &literal.text,
                                        true,
                                        TypeLiteralSource::Supplier,
                                    );
                                }
                            }
                        }
                        index_type_literals(
                            &mut index.type_literals,
                            position,
                            &value.enum_values,
                            TypeLiteralSource::EnumConstant,
                            false,
                            &parts.plural_rules,
                        );
                    }
                }
                Syntax::Function(value) => {
                    index
                        .functions_by_name
                        .entry(value.name.clone())
                        .or_default()
                        .push(position);
                }
                _ => {}
            }
        }

        if let Some(item_type) = index.types_by_code_name.get("itemtype").copied() {
            for alias in parts.aliases.aliases.keys() {
                index_type_literal(
                    &mut index.type_literals,
                    item_type,
                    None,
                    alias,
                    alias,
                    false,
                    TypeLiteralSource::Alias,
                );
                let plural = parts.plural_rules.to_plural(alias);
                if plural != *alias {
                    index_type_literal(
                        &mut index.type_literals,
                        item_type,
                        None,
                        &plural,
                        alias,
                        true,
                        TypeLiteralSource::Alias,
                    );
                }
            }
        }
        for positions in index.type_literals.values_mut() {
            positions.sort_unstable_by(|left, right| {
                let left_order = match &parts.syntaxes[left.type_position] {
                    Syntax::Type(value) => value.type_parse_order,
                    _ => usize::MAX,
                };
                let right_order = match &parts.syntaxes[right.type_position] {
                    Syntax::Type(value) => value.type_parse_order,
                    _ => usize::MAX,
                };
                left_order
                    .cmp(&right_order)
                    .then_with(|| left.source.cmp(&right.source))
                    .then_with(|| left.canonical_value.cmp(&right.canonical_value))
                    .then_with(|| left.plural.cmp(&right.plural))
            });
            positions.dedup();
        }

        for (position, event_value) in parts.event_values.iter().enumerate() {
            index
                .event_values_by_event_class
                .entry(event_value.event_class.as_str().to_owned())
                .or_default()
                .push(position);
        }

        for (position, converter) in parts.converters.iter().enumerate() {
            index
                .converters_by_from
                .entry(converter.from.as_str().to_owned())
                .or_default()
                .push(position);
            index
                .converters_by_to
                .entry(converter.to.as_str().to_owned())
                .or_default()
                .push(position);
        }

        for (position, class) in parts.classes.iter().enumerate() {
            index
                .classes_by_name
                .insert(class.name.as_str().to_owned(), position);
        }

        Self {
            parts,
            index,
            source: None,
        }
    }

    /// Attaches caller-validated source documents used to build this catalog.
    ///
    /// This does not prove that the normalized values came from `source`. Production callers
    /// should use `ssg::load`; this escape hatch exists for tests and other validated producers.
    pub fn with_unchecked_source(mut self, source: CatalogSource) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    /// Returns the opaque source snapshot when this catalog came from SSG.
    pub fn source(&self) -> Option<&CatalogSource> {
        self.source.as_deref()
    }

    /// Returns every syntax in generator registration order.
    pub fn syntaxes(&self) -> &[Syntax] {
        &self.parts.syntaxes
    }

    /// Looks up a syntax by its catalog position.
    pub fn syntax_at(&self, index: usize) -> Option<&Syntax> {
        self.parts.syntaxes.get(index)
    }

    /// Returns every syntax carrying an exact registration ID.
    pub fn syntax_by_registration_id(&self, id: &str) -> Vec<&Syntax> {
        self.index
            .syntaxes_by_registration_id
            .get(id)
            .into_iter()
            .flatten()
            .map(|position| &self.parts.syntaxes[*position])
            .collect()
    }

    /// Iterates registered events in catalog order.
    pub fn events(&self) -> impl Iterator<Item = &crate::Event> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Event(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered conditions in catalog order.
    pub fn conditions(&self) -> impl Iterator<Item = &crate::Condition> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Condition(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered effects in catalog order.
    pub fn effects(&self) -> impl Iterator<Item = &crate::Effect> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Effect(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered expressions in catalog order.
    pub fn expressions(&self) -> impl Iterator<Item = &crate::Expression> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Expression(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered Skript types in catalog order.
    pub fn types(&self) -> impl Iterator<Item = &Type> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Type(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered functions in catalog order.
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Function(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered sections in catalog order.
    pub fn sections(&self) -> impl Iterator<Item = &crate::Section> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Section(value) => Some(value),
                _ => None,
            })
    }

    /// Iterates registered structures in catalog order.
    pub fn structures(&self) -> impl Iterator<Item = &crate::Structure> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Structure(value) => Some(value),
                _ => None,
            })
    }

    /// Looks up a type by its normalized Skript code name.
    pub fn type_by_code_name(&self, code_name: &str) -> Option<&Type> {
        let position = *self.index.types_by_code_name.get(code_name)?;
        match &self.parts.syntaxes[position] {
            Syntax::Type(value) => Some(value),
            _ => None,
        }
    }

    /// Returns types whose finite parser data accepts `text`, in Skript parse order.
    pub fn type_literal_matches(&self, text: &str) -> impl Iterator<Item = TypeLiteralMatch<'_>> {
        self.index
            .type_literals
            .get(&normalize_literal(text))
            .into_iter()
            .flatten()
            .filter_map(|entry| match &self.parts.syntaxes[entry.type_position] {
                Syntax::Type(value) => Some(TypeLiteralMatch {
                    type_info: value,
                    literal: entry
                        .literal_position
                        .and_then(|position| value.type_literals.get(position)),
                    literal_index: entry.literal_position,
                    canonical_value: entry.canonical_value.as_str(),
                    plural: entry.plural,
                    source: entry.source,
                }),
                _ => None,
            })
    }

    /// Returns every type accepting `text`, preserving the compatibility API.
    pub fn type_literals(&self, text: &str) -> impl Iterator<Item = &Type> {
        self.type_literal_matches(text)
            .map(|matched| matched.type_info)
    }

    /// Returns overloads with the requested function name.
    pub fn functions_named(&self, name: &str) -> Vec<&Function> {
        self.index
            .functions_by_name
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|position| match &self.parts.syntaxes[*position] {
                Syntax::Function(value) => Some(value),
                _ => None,
            })
            .collect()
    }

    /// Returns all generated event-value registrations.
    pub fn event_values(&self) -> &[EventValue] {
        &self.parts.event_values
    }

    /// Returns inherited EventValue candidates before per-registration validation.
    ///
    /// Semantic hosts need the excluded candidates as well because native
    /// Skript treats an exclusion as an abort, not as an absent registration.
    pub fn event_value_candidates_for(&self, event_class: &str) -> Vec<&EventValue> {
        let mut positions = self
            .parts
            .event_values
            .iter()
            .enumerate()
            .filter(|(_, value)| {
                self.is_class_assignable(event_class, value.event_class.as_str())
                    || self.is_class_assignable(value.event_class.as_str(), event_class)
            })
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        positions
            .sort_unstable_by_key(|position| self.parts.event_values[*position].resolution_order);
        positions
            .into_iter()
            .map(|position| &self.parts.event_values[position])
            .collect()
    }

    /// Resolves event values inherited by an event class, honoring exclusions and order.
    pub fn event_values_for(&self, event_class: &str) -> Vec<&EventValue> {
        self.event_value_candidates_for(event_class)
            .into_iter()
            .filter(|value| {
                !value.excludes.as_ref().is_some_and(|excludes| {
                    excludes
                        .iter()
                        .any(|excluded| self.is_class_assignable(event_class, excluded.as_str()))
                })
            })
            .collect()
    }

    /// Returns all registered converters.
    pub fn converters(&self) -> &[Converter] {
        &self.parts.converters
    }

    /// Returns converters accepting the requested source class.
    pub fn converters_from(&self, class_name: &str) -> Vec<&Converter> {
        self.index
            .converters_by_from
            .get(class_name)
            .into_iter()
            .flatten()
            .map(|position| &self.parts.converters[*position])
            .collect()
    }

    /// Returns converters producing the requested target class.
    pub fn converters_to(&self, class_name: &str) -> Vec<&Converter> {
        self.index
            .converters_by_to
            .get(class_name)
            .into_iter()
            .flatten()
            .map(|position| &self.parts.converters[*position])
            .collect()
    }

    /// Tests whether Skript can pass a value from `from` to `to`.
    ///
    /// This mirrors `Converters.converterExists`: ordinary Java assignability
    /// is tried first, followed by the registered converter set and its
    /// chaining flags. SSG collects the generated chained converters after
    /// Skript finishes registration, so no runtime classes need to be loaded.
    pub fn can_convert(&self, from: &str, to: &str) -> bool {
        const NO_LEFT_CHAINING: i32 = 1;
        const NO_RIGHT_CHAINING: i32 = 2;
        const ALLOW_UNSAFE_CASTS: i32 = 4;

        if self.is_class_assignable(from, to) {
            return true;
        }
        // Skript defers Object conversions until the runtime value is known.
        if from == "java.lang.Object" {
            return true;
        }

        if self
            .parts
            .converters
            .iter()
            .any(|converter| converter.from.as_str() == from && converter.to.as_str() == to)
        {
            return true;
        }
        if self.parts.converters.iter().any(|converter| {
            self.is_class_assignable(from, converter.from.as_str())
                && self.is_class_assignable(converter.to.as_str(), to)
        }) {
            return true;
        }

        self.parts.converters.iter().any(|converter| {
            let unsafe_casts = converter.flags & ALLOW_UNSAFE_CASTS != 0;
            let narrowed_output = self.is_class_assignable(from, converter.from.as_str())
                && self.is_class_assignable(to, converter.to.as_str())
                && (unsafe_casts || converter.flags & NO_RIGHT_CHAINING == 0);
            let narrowed_input = self.is_class_assignable(converter.from.as_str(), from)
                && self.is_class_assignable(converter.to.as_str(), to)
                && (unsafe_casts || converter.flags & NO_LEFT_CHAINING == 0);
            let narrowed_both = self.is_class_assignable(converter.from.as_str(), from)
                && self.is_class_assignable(to, converter.to.as_str())
                && (unsafe_casts || converter.flags & (NO_LEFT_CHAINING | NO_RIGHT_CHAINING) == 0);
            narrowed_output || narrowed_input || narrowed_both
        })
    }

    /// Looks up one generated Java class node.
    pub fn class(&self, class_name: &str) -> Option<&Class> {
        self.index
            .classes_by_name
            .get(class_name)
            .map(|position| &self.parts.classes[*position])
    }

    /// Returns the complete Java class hierarchy captured by SSG.
    pub fn classes(&self) -> &[Class] {
        &self.parts.classes
    }

    /// Replays Skript's `Class.getDeclaredMethod` feature probe.
    ///
    /// `None` means either the class or declared-method metadata is unavailable.
    pub fn declared_method_exists(
        &self,
        class_name: &str,
        method_name: &str,
        parameter_types: &[&str],
        return_type: Option<&str>,
    ) -> Option<bool> {
        let methods = self.class(class_name)?.methods.as_ref()?;
        Some(methods.iter().any(|method| {
            method.name == method_name
                && method
                    .parameter_types
                    .iter()
                    .map(ClassName::as_str)
                    .eq(parameter_types.iter().copied())
                && return_type.is_none_or(|expected| method.return_type.as_str() == expected)
        }))
    }

    /// Tests generated Java assignability from `from` to `to` without loading classes.
    ///
    /// The traversal follows the generated superclass and interface graph and
    /// is cycle-safe. As in Java, every known non-primitive class, interface,
    /// and array is assignable to `java.lang.Object`. Unknown classes are not
    /// assumed assignable, except that a class is always assignable to itself.
    ///
    /// # Examples
    ///
    /// ~~~
    /// use syntaxes::Catalog;
    ///
    /// fn accepts_event(catalog: &Catalog, class_name: &str) -> bool {
    ///     catalog.is_class_assignable(class_name, "org.bukkit.event.Event")
    /// }
    /// # let _ = accepts_event;
    /// ~~~
    pub fn is_class_assignable(&self, from: &str, to: &str) -> bool {
        if from == to {
            return true;
        }
        if to == "java.lang.Object"
            && self
                .class(from)
                .is_some_and(|class| class.kind != ClassKind::Primitive)
        {
            return true;
        }

        let mut pending = VecDeque::from([from]);
        let mut visited = HashSet::new();
        while let Some(current) = pending.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            let Some(class) = self.class(current) else {
                continue;
            };
            let parents = class
                .super_class
                .iter()
                .chain(class.interfaces.iter())
                .map(|parent| parent.as_str());
            for parent in parents {
                if parent == to {
                    return true;
                }
                pending.push_back(parent);
            }
        }
        false
    }

    /// Returns Skript's superclass-chain distance from `subclass` to `superclass`.
    ///
    /// This mirrors `ClassUtils.hierarchyDistance`: Java assignability is checked
    /// first, then only the concrete superclass chain contributes to the distance.
    /// Implemented interfaces therefore use the depth from the class to the end
    /// of that chain, matching Skript's runtime comparator.
    pub fn hierarchy_distance(&self, superclass: &str, subclass: &str) -> Option<u64> {
        if !self.is_class_assignable(subclass, superclass) {
            return None;
        }
        if superclass == subclass {
            return Some(0);
        }

        let mut distance = 0_u64;
        let mut current = Some(subclass);
        while let Some(class_name) = current {
            if class_name == superclass {
                break;
            }
            let class = self.class(class_name)?;
            current = class.super_class.as_ref().map(ClassName::as_str);
            distance = distance.saturating_add(1);
        }
        Some(distance)
    }

    /// Finds the Skript-compatible common Java type for two captured classes.
    ///
    /// Concrete superclasses are preferred over interfaces. When only unrelated
    /// common interfaces remain, the selected interface follows Java declaration
    /// order, matching Skript's `Utils.highestDenominator` behavior.
    pub fn common_assignable_class(&self, left: &str, right: &str) -> Option<ClassName> {
        self.common_assignable_class_inner(left, right, &mut HashSet::new())
    }

    /// Finds the common Java type for every class in a non-empty list.
    pub fn common_assignable_classes(&self, classes: &[ClassName]) -> Option<ClassName> {
        let first = classes.first()?.clone();
        classes.iter().skip(1).try_fold(first, |common, class| {
            self.common_assignable_class(common.as_str(), class.as_str())
        })
    }

    /// Finds the registered Skript type class used for a list of Java return types.
    ///
    /// Skript first computes the Java common type, then resolves its exact or
    /// first assignable `ClassInfo` in registration order.
    pub fn common_skript_class(&self, classes: &[ClassName]) -> Option<ClassName> {
        let common = self.common_assignable_classes(classes)?;
        self.types()
            .find(|ty| ty.original_class == common)
            .or_else(|| {
                self.types()
                    .enumerate()
                    .filter(|(_, ty)| {
                        self.is_class_assignable(common.as_str(), ty.original_class.as_str())
                    })
                    .min_by_key(|(index, ty)| (ty.type_parse_order, *index))
                    .map(|(_, ty)| ty)
            })
            .map(|ty| ty.original_class.clone())
    }

    fn common_assignable_class_inner(
        &self,
        left: &str,
        right: &str,
        visited: &mut HashSet<(String, String)>,
    ) -> Option<ClassName> {
        if !visited.insert((left.to_owned(), right.to_owned())) {
            return None;
        }
        if self.is_class_assignable(right, left) {
            return Some(common_class_result(left));
        }

        let mut current = Some(right);
        let mut superclasses = HashSet::new();
        while let Some(candidate) = current {
            if !superclasses.insert(candidate) {
                break;
            }
            if candidate != "java.lang.Object" && self.is_class_assignable(left, candidate) {
                return Some(common_class_result(candidate));
            }
            current = self
                .class(candidate)
                .and_then(|class| class.super_class.as_ref())
                .map(ClassName::as_str);
        }

        for interface in &self.class(right)?.interfaces {
            let mut branch = visited.clone();
            if let Some(common) =
                self.common_assignable_class_inner(interface.as_str(), left, &mut branch)
                && common.as_str() != "java.lang.Object"
            {
                return Some(common);
            }
        }

        (self.is_class_assignable(left, "java.lang.Object")
            && self.is_class_assignable(right, "java.lang.Object"))
        .then(|| ClassName("java.lang.Object".to_owned()))
    }

    /// Tests assignability between the Java classes represented by two Skript types.
    pub fn is_type_assignable(&self, from: &str, to: &str) -> bool {
        from == to
            || self.type_by_code_name(from).is_some_and(|source| {
                source
                    .assignable_to
                    .iter()
                    .any(|target| target.as_str() == to)
            })
    }

    /// Resolves an alias using Skript-compatible case-insensitive lookup.
    pub fn alias(&self, text: &str) -> Option<&crate::AliasTarget> {
        self.parts
            .aliases
            .aliases
            .get(&normalize_literal(text))
            .and_then(|index| self.parts.aliases.targets.get(*index))
    }

    /// Returns the complete normalized alias registry.
    pub fn aliases(&self) -> &AliasRegistry {
        &self.parts.aliases
    }

    /// Returns all registered comparator handlers.
    pub fn comparators(&self) -> &[Comparator] {
        &self.parts.comparators
    }

    /// Returns all registered properties.
    pub fn properties(&self) -> &[Property] {
        &self.parts.properties
    }

    /// Returns all registered arithmetic operators.
    pub fn operators(&self) -> &[Operator] {
        &self.parts.operators
    }

    /// Returns arithmetic operations grouped by operator ID.
    pub fn operations(&self) -> &BTreeMap<String, Vec<Operation>> {
        &self.parts.operations
    }

    /// Returns all registered difference handlers.
    pub fn differences(&self) -> &[Difference] {
        &self.parts.differences
    }

    /// Returns difference handlers whose registered input accepts `input_class`.
    ///
    /// Exact handlers come first, followed by the closest superclass handlers
    /// and registration order. Returning every candidate lets semantic addons
    /// preserve uncertainty instead of baking one runtime policy into the host.
    pub fn difference_options_for_type(&self, input_class: &str) -> Vec<&Difference> {
        let mut differences = self
            .parts
            .differences
            .iter()
            .filter(|difference| {
                self.is_class_assignable(input_class, difference.input_type.as_str())
            })
            .collect::<Vec<_>>();
        differences.sort_by_key(|difference| {
            (
                difference.input_type.as_str() != input_class,
                self.hierarchy_distance(difference.input_type.as_str(), input_class)
                    .unwrap_or(u64::MAX),
                difference.registration_order,
            )
        });
        differences
    }

    /// Returns the exact server-specific plural conversion rules.
    pub fn plural_rules(&self) -> &PluralRules {
        &self.parts.plural_rules
    }

    /// Returns the exact runtime-localized value for a language key.
    ///
    /// Keys are case-sensitive because Skript's language registry treats them as
    /// resource identifiers, not user-facing words. An absent key is distinct
    /// from a present key whose value is an empty string.
    pub fn language_value(&self, key: &str) -> Option<&str> {
        self.parts.language.get(key).map(String::as_str)
    }

    /// Iterates effective runtime language entries in deterministic key order.
    pub fn language_entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.parts
            .language
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

fn source_digest(documents: &BTreeMap<String, Arc<[u8]>>) -> String {
    let mut digest = Sha256::new();
    for (name, bytes) in documents {
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn common_class_result(class_name: &str) -> ClassName {
    ClassName(
        if class_name == "java.lang.Cloneable" {
            "java.lang.Object"
        } else {
            class_name
        }
        .to_owned(),
    )
}

fn normalize_literal(text: &str) -> String {
    text.trim().to_lowercase()
}
