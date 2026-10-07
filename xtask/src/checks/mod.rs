//! Architecture checks run by `cargo xtask ci`. Each check is a pure function
//! over inputs gathered here, so its tests can feed it failing fixtures.

pub mod ignore;
pub mod layering;
pub mod purity;
pub mod size;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, Package, PackageId};

const PREFIX: &str = "nomi-";

/// Runs every check and returns the violations, sorted.
pub fn run(root: &Path) -> Result<Vec<String>> {
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .exec()
        .context("running cargo metadata")?;
    let mut violations = Vec::new();

    let nomi: Vec<&Package> = metadata
        .workspace_packages()
        .into_iter()
        .filter(|p| p.name.starts_with(PREFIX))
        .collect();
    let crates: Vec<String> = nomi.iter().map(|p| short(&p.name).to_string()).collect();
    let edges: Vec<(String, String)> = nomi
        .iter()
        .flat_map(|p| {
            p.dependencies
                .iter()
                .filter(|d| d.name.starts_with(PREFIX))
                .map(|d| (short(&p.name).to_string(), short(&d.name).to_string()))
        })
        .collect();
    violations.extend(layering::check(&crates, &edges));

    let closures: Vec<(String, Vec<String>)> = nomi
        .iter()
        .filter(|p| purity::PURE.contains(&short(&p.name)))
        .map(|p| {
            (
                short(&p.name).to_string(),
                external_closure(&metadata, &p.id),
            )
        })
        .collect();
    violations.extend(purity::check_dependencies(&closures));

    for package in &nomi {
        if !purity::PURE.contains(&short(&package.name)) {
            continue;
        }
        let src = package.manifest_path.parent().unwrap().join("src");
        for file in rust_files(src.as_std_path())? {
            let source = read(&file)?;
            violations.extend(purity::scan_source(&label(root, &file), &source));
        }
    }

    for dir in ["crates", "xtask"] {
        for file in rust_files(&root.join(dir))? {
            let source = read(&file)?;
            let label = label(root, &file);
            violations.extend(size::check(&label, &source));
            violations.extend(ignore::check(&label, &source));
        }
    }

    violations.sort();
    Ok(violations)
}

fn short(name: &str) -> &str {
    name.strip_prefix(PREFIX).unwrap_or(name)
}

/// Names of the non-nomi packages reachable from `id` through normal
/// dependencies. Build and dev dependencies do not ship in the crate.
fn external_closure(metadata: &Metadata, id: &PackageId) -> Vec<String> {
    let resolve = metadata.resolve.as_ref().expect("cargo metadata resolves");
    let nodes: BTreeMap<&PackageId, _> = resolve.nodes.iter().map(|n| (&n.id, n)).collect();
    let mut seen = BTreeSet::new();
    let mut stack = vec![id];
    let mut external = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        for dep in &nodes[id].deps {
            if !dep
                .dep_kinds
                .iter()
                .any(|k| k.kind == DependencyKind::Normal)
            {
                continue;
            }
            let name = metadata[&dep.pkg].name.to_string();
            if !name.starts_with(PREFIX) {
                external.insert(name);
            }
            stack.push(&dep.pkg);
        }
    }
    external.into_iter().collect()
}

fn rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !size::SKIPPED_DIRS.contains(&name) {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn label(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}
