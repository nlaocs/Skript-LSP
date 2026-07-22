use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use wit_component::{ComponentEncoder, DecodedWasm};

const TARGET: &str = "wasm32-unknown-unknown";
const PROFILE: &str = "core-library";
const ARTIFACT_NAME: &str = "core-library.wasm";

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("build-core-library") => build_core_library(),
        Some(command) => bail!("unknown xtask command {command:?}"),
        None => bail!("usage: cargo run -p xtask -- build-core-library"),
    }
}

fn build_core_library() -> Result<()> {
    let root = workspace_root()?;
    let target_dir = root.join("target").join("core-library-component");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let status = Command::new(cargo)
        .current_dir(&root)
        .args([
            "build",
            "--locked",
            "--package",
            "core-library",
            "--target",
            TARGET,
            "--profile",
            PROFILE,
            "--target-dir",
        ])
        .arg(&target_dir)
        .status()
        .context("failed to start Cargo for CoreLibrary")?;
    if !status.success() {
        bail!("CoreLibrary core Wasm build failed with {status}");
    }

    let module_path = target_dir
        .join(TARGET)
        .join(PROFILE)
        .join("core_library.wasm");
    let module = fs::read(&module_path).with_context(|| {
        format!(
            "CoreLibrary core Wasm is missing: {}",
            module_path.display()
        )
    })?;
    let component = ComponentEncoder::default()
        .module(&module)
        .context("CoreLibrary module has invalid component metadata")?
        .validate(true)
        .encode()
        .context("failed to encode CoreLibrary component")?;
    validate_component(&component)?;

    let artifact_dir = root.join("artifacts");
    fs::create_dir_all(&artifact_dir).context("failed to create artifact directory")?;
    let artifact = artifact_dir.join(ARTIFACT_NAME);
    let temporary = artifact_dir.join(format!("{ARTIFACT_NAME}.tmp"));
    fs::write(&temporary, component).context("failed to write temporary CoreLibrary artifact")?;
    if artifact.exists() {
        fs::remove_file(&artifact).context("failed to replace old CoreLibrary artifact")?;
    }
    fs::rename(&temporary, &artifact).context("failed to publish CoreLibrary artifact")?;

    println!("CoreLibrary component: {}", artifact.display());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("xtask must be located directly under the workspace root")
}

fn validate_component(component: &[u8]) -> Result<()> {
    let DecodedWasm::Component(resolve, world_id) =
        wit_component::decode(component).context("generated artifact is not valid Wasm")?
    else {
        bail!("generated CoreLibrary artifact is not a Component");
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
        bail!("CoreLibrary exports {exports:?}, expected {expected:?}");
    }
    Ok(())
}
