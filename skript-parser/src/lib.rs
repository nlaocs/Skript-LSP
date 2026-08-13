#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

mod arithmetic;
mod catalog_match;
mod condition;
mod effect;
mod expansion;
mod expression;
mod function;
mod pattern_match;
mod raw_tree;
mod section;
mod source_map;
mod text;
mod tree_edit;

/// Adapters from static and dynamic syntax catalogs to matcher candidates.
pub use catalog_match::*;
/// Condition parsing over SSG registrations and recursive Expressions.
pub use condition::*;
/// Effect parsing over lossless Simple nodes and recursive Expressions.
pub use effect::*;
/// Macro expansion identities, syntax contexts, and provenance graph.
pub use expansion::*;
/// Recursive Expression parsing over SSG registrations and parser extensions.
pub use expression::*;
/// Registered and future document-defined Function call parsing.
pub use function::*;
/// Registered-pattern matching and typed extension points.
pub use pattern_match::*;
/// Lossless indentation tree and recoverable lexical diagnostics.
pub use raw_tree::*;
/// Recursive Section parsing over RawTree headers and bodies.
pub use section::*;
/// Original/virtual source mapping and validated text transformations.
pub use source_map::*;
/// UTF-8 byte range primitives shared by every parser stage.
pub use text::*;
/// Validated transformations over lossless raw trees.
pub use tree_edit::*;
