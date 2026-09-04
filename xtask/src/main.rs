//! Workspace build orchestration for parser-addon WebAssembly Components.
//!
//! Commands compile guest crates, convert core modules to Component Model artifacts,
//! validate exports, and place generated files in `artifacts/`.

use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use wit_component::{ComponentEncoder, DecodedWasm};

const TARGET: &str = "wasm32-unknown-unknown";
const PROFILE: &str = "core-library";

struct ComponentSpec {
    package: &'static str,
    module_name: &'static str,
    artifact_name: &'static str,
    display_name: &'static str,
    feature: Option<&'static str>,
}

const CORE_LIBRARY: ComponentSpec = ComponentSpec {
    package: "core-library",
    module_name: "core_library.wasm",
    artifact_name: "core-library.wasm",
    display_name: "CoreLibrary",
    feature: None,
};

const DYNAMIC_SYNTAX_ADDON: ComponentSpec = ComponentSpec {
    package: "dynamic-syntax-addon",
    module_name: "dynamic_syntax_addon.wasm",
    artifact_name: "dynamic-syntax-addon.wasm",
    display_name: "dynamic syntax test addon",
    feature: None,
};

const CATALOG_DATA_ADDON: ComponentSpec = ComponentSpec {
    package: "catalog-data-addon",
    module_name: "catalog_data_addon.wasm",
    artifact_name: "catalog-data-addon.wasm",
    display_name: "Catalog Data test addon",
    feature: None,
};

const EFFECT_ADDON: ComponentSpec = ComponentSpec {
    package: "effect-addon",
    module_name: "effect_addon.wasm",
    artifact_name: "effect-addon.wasm",
    display_name: "Effect hook test addon",
    feature: None,
};
const MATCHING_ADDON: ComponentSpec = ComponentSpec {
    package: "matching-addon",
    module_name: "matching_addon.wasm",
    artifact_name: "matching-addon.wasm",
    display_name: "matching hook test addon",
    feature: None,
};

const TEXT_MACRO_ADDON: ComponentSpec = ComponentSpec {
    package: "text-macro-addon",
    module_name: "text_macro_addon.wasm",
    artifact_name: "text-macro-addon.wasm",
    display_name: "text macro test addon",
    feature: None,
};
const TREE_MACRO_ADDON: ComponentSpec = ComponentSpec {
    package: "tree-macro-addon",
    module_name: "tree_macro_addon.wasm",
    artifact_name: "tree-macro-addon.wasm",
    display_name: "tree macro test addon",
    feature: None,
};

const EXPRESSION_DATA_ADDON_A: ComponentSpec = ComponentSpec {
    package: "expression-data-addon",
    module_name: "expression_data_addon.wasm",
    artifact_name: "expression-data-addon-a.wasm",
    display_name: "expression data test addon A",
    feature: None,
};

const EXPRESSION_DATA_ADDON_B: ComponentSpec = ComponentSpec {
    package: "expression-data-addon",
    module_name: "expression_data_addon.wasm",
    artifact_name: "expression-data-addon-b.wasm",
    display_name: "expression data test addon B",
    feature: Some("addon-b"),
};

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("build-core-library") => build_core_library(),
        Some("build-test-components") => build_test_components(),
        Some(command) => bail!("unknown xtask command {command:?}"),
        None => bail!("usage: cargo run -p xtask -- <build-core-library|build-test-components>"),
    }
}

fn build_core_library() -> Result<()> {
    build_components(&[&CORE_LIBRARY])
}

fn build_test_components() -> Result<()> {
    build_components(&[
        &CATALOG_DATA_ADDON,
        &DYNAMIC_SYNTAX_ADDON,
        &EFFECT_ADDON,
        &MATCHING_ADDON,
        &TEXT_MACRO_ADDON,
        &TREE_MACRO_ADDON,
    ])?;
    build_components(&[&EXPRESSION_DATA_ADDON_A])?;
    build_components(&[&EXPRESSION_DATA_ADDON_B])
}

fn build_components(specs: &[&ComponentSpec]) -> Result<()> {
    let first = specs
        .first()
        .context("at least one parser addon component must be requested")?;
    let root = workspace_root()?;
    let target_root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let target_dir = if specs.len() == 1 {
        target_root.join(format!("{}-component", first.package))
    } else {
        target_root.join("test-components")
    };
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command.current_dir(&root).args(["build", "--locked"]);
    for spec in specs {
        command.args(["--package", spec.package]);
    }
    if specs.len() == 1
        && let Some(feature) = first.feature
    {
        command.args(["--no-default-features", "--features", feature]);
    }
    let status = command
        .args(["--target", TARGET, "--profile", PROFILE, "--target-dir"])
        .arg(&target_dir)
        .status()
        .context("failed to start Cargo for parser addon components")?;
    if !status.success() {
        bail!("parser addon component build failed with {status}");
    }

    for spec in specs {
        publish_component(&root, &target_dir, spec)?;
    }
    Ok(())
}

fn publish_component(root: &Path, target_dir: &Path, spec: &ComponentSpec) -> Result<()> {
    let module_path = target_dir.join(TARGET).join(PROFILE).join(spec.module_name);
    let module = fs::read(&module_path).with_context(|| {
        format!(
            "{} core Wasm is missing: {}",
            spec.display_name,
            module_path.display()
        )
    })?;
    let component = ComponentEncoder::default()
        .module(&module)
        .with_context(|| {
            format!(
                "{} module has invalid component metadata",
                spec.display_name
            )
        })?
        .validate(true)
        .encode()
        .with_context(|| format!("failed to encode {} component", spec.display_name))?;
    validate_component(&component, spec.display_name)?;

    let artifact_dir = root.join("artifacts");
    fs::create_dir_all(&artifact_dir).context("failed to create artifact directory")?;
    let artifact = artifact_dir.join(spec.artifact_name);
    let temporary = artifact_dir.join(format!("{}.tmp", spec.artifact_name));
    fs::write(&temporary, component)
        .with_context(|| format!("failed to write temporary {} artifact", spec.display_name))?;
    if artifact.exists() {
        fs::remove_file(&artifact)
            .with_context(|| format!("failed to replace old {} artifact", spec.display_name))?;
    }
    fs::rename(&temporary, &artifact)
        .with_context(|| format!("failed to publish {} artifact", spec.display_name))?;

    println!("{} component: {}", spec.display_name, artifact.display());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("xtask must be located directly under the workspace root")
}

fn validate_component(component: &[u8], display_name: &str) -> Result<()> {
    let DecodedWasm::Component(resolve, world_id) =
        wit_component::decode(component).context("generated artifact is not valid Wasm")?
    else {
        bail!("generated {display_name} artifact is not a Component");
    };
    let world = &resolve.worlds[world_id];
    let exports = world
        .exports
        .values()
        .filter_map(|item| match item {
            wit_parser::WorldItem::Interface { id, .. } => resolve.interfaces[*id].name.as_deref(),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from(["addon", "hooks", "text-macro", "tree-macro", "ast-macro"]);
    if exports != expected {
        bail!("{display_name} exports {exports:?}, expected {expected:?}");
    }
    Ok(())
}
