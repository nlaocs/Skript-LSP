#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

//! The usual entry point is [`load`]. Consumers that need to inspect the JSON
//! format itself can use [`raw`], while parser and LSP code should use the
//! normalized [`syntaxes::Catalog`] returned by [`Snapshot::catalog`].

mod convert;
mod digest;
mod error;
mod loader;
/// Serde data-transfer objects that mirror SSG schema 3 JSON.
///
/// These types preserve the generator's wire format, including nullable
/// resolution states. They are public for format tooling; runtime consumers
/// should prefer the normalized `syntaxes` model.
#[allow(missing_docs)]
pub mod raw;
mod validate;

pub use error::SnapshotError;
pub use loader::{ALL_FILES, DATA_FILES, SCHEMA_VERSION, Snapshot, load};
