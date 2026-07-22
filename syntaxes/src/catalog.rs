use crate::{
    AliasRegistry, Class, Comparator, Converter, Difference, EventValue, Function, Operation,
    Operator, Property, Syntax, Type,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use syntax_pattern_parser::syntax::PluralRules;

#[derive(Debug, Clone)]
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
pub struct Catalog {
    parts: CatalogParts,
    index: CatalogIndex,
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
}

impl Catalog {
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
                }
                Syntax::Function(value) => {
                    if let Some(name) = &value.documentation.name {
                        index
                            .functions_by_name
                            .entry(name.clone())
                            .or_default()
                            .push(position);
                    }
                }
                _ => {}
            }
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

    pub fn syntaxes(&self) -> &[Syntax] {
        &self.parts.syntaxes
    }

    pub fn syntax_by_registration_id(&self, id: &str) -> Vec<&Syntax> {
        self.index
            .syntaxes_by_registration_id
            .get(id)
            .into_iter()
            .flatten()
            .map(|position| &self.parts.syntaxes[*position])
            .collect()
    }

    pub fn events(&self) -> impl Iterator<Item = &crate::Event> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Event(value) => Some(value),
                _ => None,
            })
    }

    pub fn conditions(&self) -> impl Iterator<Item = &crate::Condition> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Condition(value) => Some(value),
                _ => None,
            })
    }

    pub fn effects(&self) -> impl Iterator<Item = &crate::Effect> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Effect(value) => Some(value),
                _ => None,
            })
    }

    pub fn expressions(&self) -> impl Iterator<Item = &crate::Expression> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Expression(value) => Some(value),
                _ => None,
            })
    }

    pub fn types(&self) -> impl Iterator<Item = &Type> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Type(value) => Some(value),
                _ => None,
            })
    }

    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Function(value) => Some(value),
                _ => None,
            })
    }

    pub fn sections(&self) -> impl Iterator<Item = &crate::Section> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Section(value) => Some(value),
                _ => None,
            })
    }

    pub fn structures(&self) -> impl Iterator<Item = &crate::Structure> {
        self.parts
            .syntaxes
            .iter()
            .filter_map(|syntax| match syntax {
                Syntax::Structure(value) => Some(value),
                _ => None,
            })
    }

    pub fn type_by_code_name(&self, code_name: &str) -> Option<&Type> {
        let position = *self.index.types_by_code_name.get(code_name)?;
        match &self.parts.syntaxes[position] {
            Syntax::Type(value) => Some(value),
            _ => None,
        }
    }

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

    pub fn event_values(&self) -> &[EventValue] {
        &self.parts.event_values
    }

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

    pub fn converters(&self) -> &[Converter] {
        &self.parts.converters
    }

    pub fn converters_from(&self, class_name: &str) -> Vec<&Converter> {
        self.index
            .converters_by_from
            .get(class_name)
            .into_iter()
            .flatten()
            .map(|position| &self.parts.converters[*position])
            .collect()
    }

    pub fn converters_to(&self, class_name: &str) -> Vec<&Converter> {
        self.index
            .converters_by_to
            .get(class_name)
            .into_iter()
            .flatten()
            .map(|position| &self.parts.converters[*position])
            .collect()
    }

    pub fn class(&self, class_name: &str) -> Option<&Class> {
        self.index
            .classes_by_name
            .get(class_name)
            .map(|position| &self.parts.classes[*position])
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
    pub fn is_class_assignable(&self, from: &str, to: &str) -> bool {
        if from == to {
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

    pub fn is_type_assignable(&self, from: &str, to: &str) -> bool {
        from == to
            || self.type_by_code_name(from).is_some_and(|source| {
                source
                    .assignable_to
                    .iter()
                    .any(|target| target.as_str() == to)
            })
    }

    pub fn alias(&self, text: &str) -> Option<&crate::AliasTarget> {
        self.parts
            .aliases
            .aliases
            .get(text)
            .and_then(|index| self.parts.aliases.targets.get(*index))
    }

    pub fn aliases(&self) -> &AliasRegistry {
        &self.parts.aliases
    }

    pub fn comparators(&self) -> &[Comparator] {
        &self.parts.comparators
    }

    pub fn properties(&self) -> &[Property] {
        &self.parts.properties
    }

    pub fn operators(&self) -> &[Operator] {
        &self.parts.operators
    }

    pub fn operations(&self) -> &BTreeMap<String, Vec<Operation>> {
        &self.parts.operations
    }

    pub fn differences(&self) -> &[Difference] {
        &self.parts.differences
    }

    pub fn plural_rules(&self) -> &PluralRules {
        &self.parts.plural_rules
    }
}
