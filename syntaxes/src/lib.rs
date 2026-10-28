#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

mod catalog;
mod dynamic;
mod model;
mod json_pages;

/// Bounded structural access to retained JSON array records.
pub use json_pages::{JsonArrayIndex, JsonArrayPage, JsonPageError, JsonPageRecord};

/// Indexed, immutable syntax data and semantic relationship queries.
pub use catalog::*;
/// Transactional dynamic syntax registration, overrides, and ranking.
pub use dynamic::*;
/// Format-independent syntax and registry value types.
pub use model::*;
