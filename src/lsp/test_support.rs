//! Immutable fixture data only. Hosts, Stores, certificates and documents stay local.
use std::{
    path::Path,
    sync::{Arc, LazyLock},
};
use syntaxes::Catalog;
fn load(relative: &str) -> Arc<Catalog> {
    Arc::new(
        ssg::load(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
            .unwrap()
            .into_catalog(),
    )
}
pub(super) fn multi_addon_catalog() -> Arc<Catalog> {
    static CATALOG: LazyLock<Arc<Catalog>> =
        LazyLock::new(|| load("syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4"));
    Arc::clone(&CATALOG)
}
pub(super) fn runtime_catalog() -> Arc<Catalog> {
    static CATALOG: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
        load("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-type-runtime")
    });
    Arc::clone(&CATALOG)
}
