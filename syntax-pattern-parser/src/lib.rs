#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

mod pattern;
mod plural;

/// Parser and model for syntax registration patterns.
///
/// Events, conditions, effects, expressions, sections, and structures use
/// this grammar. Registered Skript types are catalog data and are not parsed
/// as complete patterns by this module.
pub mod syntax {
    use super::{pattern, plural};
    pub use pattern::*;
    pub use plural::*;
}
