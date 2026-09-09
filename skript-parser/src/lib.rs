#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

mod arithmetic;
mod catalog_match;
mod condition;
mod default_expression;
mod effect;
mod event;
mod expansion;
mod expression;
mod expression_list;
mod failure;
mod function;
mod function_registry;
mod pattern_match;
mod raw_tree;
mod section;
mod source_map;
mod structure;
mod text;
mod tree_edit;

/// Adapters from static and dynamic syntax catalogs to matcher candidates.
pub use catalog_match::*;
/// Condition parsing over SSG registrations and recursive Expressions.
pub use condition::*;
/// Shared default Expression requests, outcomes and implicit-node provenance.
pub use default_expression::*;
/// Effect parsing over lossless Simple nodes and recursive Expressions.
pub use effect::*;
/// Event-header matching used by StructEvent and addon Structure handlers.
pub use event::*;
/// Macro expansion identities, syntax contexts, and provenance graph.
pub use expansion::*;
/// Recursive Expression parsing over SSG registrations and parser extensions.
pub use expression::*;
/// Skript Expression-list conjunction semantics.
pub use expression_list::*;
/// Nested parse-failure provenance shared by diagnostics and LSP adapters.
pub use failure::*;
/// Registered and document-defined Function call parsing.
pub use function::*;
/// Transactional registry for document-defined Function declarations.
pub use function_registry::*;
/// Registered-pattern matching and typed extension points.
pub use pattern_match::*;
/// Lossless indentation tree and recoverable lexical diagnostics.
pub use raw_tree::*;
/// Recursive Section parsing over RawTree headers and bodies.
pub use section::*;
/// Original/virtual source mapping and validated text transformations.
pub use source_map::*;
/// Top-level Structure parsing and EntryValidator enforcement.
pub use structure::*;
/// UTF-8 byte range primitives shared by every parser stage.
pub use text::*;
/// Validated transformations over lossless raw trees.
pub use tree_edit::*;
