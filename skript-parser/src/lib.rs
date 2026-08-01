#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

mod catalog_match;
mod effect;
mod expansion;
mod expression;
mod pattern_match;
mod raw_tree;
mod source_map;
mod text;
mod tree_edit;

/// Adapters from static and dynamic syntax catalogs to matcher candidates.
pub use catalog_match::*;
/// Effect parsing over lossless Simple nodes and recursive Expressions.
pub use effect::*;
/// Macro expansion identities, syntax contexts, and provenance graph.
pub use expansion::*;
/// Recursive Expression parsing over SSG registrations and parser extensions.
pub use expression::*;
/// Registered-pattern matching and typed extension points.
pub use pattern_match::*;
/// Lossless indentation tree and recoverable lexical diagnostics.
pub use raw_tree::*;
/// Original/virtual source mapping and validated text transformations.
pub use source_map::*;
/// UTF-8 byte range primitives shared by every parser stage.
pub use text::*;
/// Validated transformations over lossless raw trees.
pub use tree_edit::*;
