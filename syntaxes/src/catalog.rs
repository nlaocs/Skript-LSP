//! Immutable indexes and semantic queries over normalized syntax data.
//!
//! Indexes preserve generator registration order and provide cycle-safe traversal
//! for Java assignability, converters, event values, functions, and aliases.
#![allow(missing_docs)] // Public fields are described by their owning domain type.

use crate::{
    AliasRegistry, Class, ClassKind, ClassName, Comparator, Converter, Difference, EventValue,
    Function, Operation, Operator, Property, Syntax, Type, TypeLiteral,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use syntax_pattern_parser::syntax::PluralRules;

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
}

#[derive(Debug, Clone)]
/// Immutable normalized registry with indexes for parser and LSP queries.
///
/// A catalog is the semantic view of one complete server snapshot. It keeps
/// generator registration order while adding indexes for syntax IDs, type code
/// names, functions, converters, event values, aliases, and Java class
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

        Self { parts, index }
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

    /// Resolves event values inherited by an event class, honoring exclusions and order.
    pub fn event_values_for(&self, event_class: &str) -> Vec<&EventValue> {
        let mut positions = self
            .class_lineage(event_class)
            .into_iter()
            .filter_map(|class| self.index.event_values_by_event_class.get(class))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        positions
            .sort_unstable_by_key(|position| self.parts.event_values[*position].resolution_order);
        positions
            .into_iter()
            .map(|position| &self.parts.event_values[position])
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

    fn class_lineage<'a>(&'a self, class_name: &'a str) -> Vec<&'a str> {
        let mut result = Vec::new();
        let mut pending = VecDeque::from([class_name]);
        let mut visited = HashSet::new();
        while let Some(current) = pending.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            result.push(current);
            if let Some(class) = self.class(current) {
                pending.extend(
                    class
                        .super_class
                        .iter()
                        .chain(class.interfaces.iter())
                        .map(|parent| parent.as_str()),
                );
            }
        }
        result
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

    /// Finds the nearest captured Java type that can hold values from both classes.
    pub fn common_assignable_class(&self, left: &str, right: &str) -> Option<ClassName> {
        self.class_lineage(left)
            .into_iter()
            .find(|candidate| self.is_class_assignable(right, candidate))
            .map(|candidate| ClassName(candidate.to_owned()))
            .or_else(|| {
                (self.is_class_assignable(left, "java.lang.Object")
                    && self.is_class_assignable(right, "java.lang.Object"))
                .then(|| ClassName("java.lang.Object".to_owned()))
            })
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

    /// Returns the exact server-specific plural conversion rules.
    pub fn plural_rules(&self) -> &PluralRules {
        &self.parts.plural_rules
    }
}

fn normalize_literal(text: &str) -> String {
    text.trim().to_lowercase()
}
