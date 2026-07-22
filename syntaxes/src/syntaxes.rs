#![allow(dead_code)]
use skripthub::addon_syntax_list::entity::{
    Condition, Effect, Event, Expression, Function, Section, Structure, Type,
};
use skripthub::api::types::{AbstractAddonSyntaxList, AbstractAddonSyntaxListEntry, SyntaxType};

macro_rules! handle_syntax {
    ($s:expr, $syntaxes:expr, $errored_syntaxes:expr, $plural_rules:expr, {
        $( $syntax_type:pat => $variant:ty => $field:ident ),* $(,)?
    }) => {
        match $s.syntax_type {
            $(
                $syntax_type => {
                    match <$variant>::from_abstract_syntax_list_entry(&$s, $plural_rules) {
                        Ok(val) => {
                            $syntaxes.$field.push(val);
                        }
                        Err(e) => {
                            $errored_syntaxes.push(($s, e));
                        }
                    }
                }
            )*
        }
    }
}

pub type SyntaxesAndErrors = (
    Syntaxes,
    Vec<(AbstractAddonSyntaxListEntry, Box<dyn std::error::Error>)>,
);

#[derive(Debug, Clone, Default)]
pub struct Syntaxes {
    pub events: Vec<Event>,
    pub conditions: Vec<Condition>,
    pub effects: Vec<Effect>,
    pub expressions: Vec<Expression>,
    pub r#types: Vec<Type>,
    pub functions: Vec<Function>,
    pub sections: Vec<Section>,
    pub structures: Vec<Structure>,
}
impl Syntaxes {
    pub fn initialize(
        plural_rules: &syntax_pattern_parser::syntax::PluralRules,
    ) -> Result<SyntaxesAndErrors, Box<dyn std::error::Error>> {
        use skripthub::api::fetch_data;
        let abstract_syntax_list = fetch_data()?;
        Ok(Self::from_abstract_syntax_list(
            abstract_syntax_list,
            plural_rules,
        ))
    }

    pub fn from_abstract_syntax_list(
        abstract_syntax_list: AbstractAddonSyntaxList,
        plural_rules: &syntax_pattern_parser::syntax::PluralRules,
    ) -> SyntaxesAndErrors {
        let mut syntaxes = Syntaxes::default();
        let mut error_syntaxes: Vec<(AbstractAddonSyntaxListEntry, Box<dyn std::error::Error>)> =
            Vec::new();
        for s in abstract_syntax_list {
            handle_syntax!(s, syntaxes, error_syntaxes, plural_rules, {
                SyntaxType::Event => Event => events,
                SyntaxType::Condition => Condition => conditions,
                SyntaxType::Effect => Effect => effects,
                SyntaxType::Expression => Expression => expressions,
                SyntaxType::Type => Type => r#types,
                SyntaxType::Function => Function => functions,
                SyntaxType::Section => Section => sections,
                SyntaxType::Structure => Structure => structures,
            });
        }
        (syntaxes, error_syntaxes)
    }
}
