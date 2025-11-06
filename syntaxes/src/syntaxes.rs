#![allow(dead_code)]
use skripthub::addon_syntax_list::entity::{
    Condition, Effect, Event, Expression, Function, Section, Structure, Type,
};
use skripthub::api::types::{AbstractAddonSyntaxList, SyntaxType};

macro_rules! handle_syntax {
    ($s:expr, $syntaxes:expr, {
        $( $syntax_type:pat => $variant:ty => $field:ident ),* $(,)?
    }) => {
        match $s.syntax_type {
            $(
                $syntax_type => {
                    if let Ok(val) = <$variant>::from_abstract_syntax_list_entry($s) {
                        $syntaxes.$field.push(val);
                    }
                }
            )*
        }
    }
}

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
    pub fn initialize() -> Result<Self, Box<dyn std::error::Error>> {
        use skripthub::api::fetch_data;
        let abstract_syntax_list = fetch_data()?;
        Ok(Self::from_abstract_syntax_list(abstract_syntax_list))
    }
    pub fn from_abstract_syntax_list(abstract_syntax_list: AbstractAddonSyntaxList) -> Self {
        let mut syntaxes = Syntaxes::default();
        for s in abstract_syntax_list {
            handle_syntax!(s, syntaxes, {
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
        syntaxes
    }
}
