//! `cargo xtask oracle`: the pinned upstream tools that parity is measured
//! against, installed under the development cache and never on `PATH`.

pub mod format;
pub mod sandbox;
pub mod syntax;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::download;
use crate::references::{Binary, Format, References, platform};

/// `$NOMI_DEV_CACHE`, or `~/.cache/nomi-dev`.
pub fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("NOMI_DEV_CACHE") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("neither NOMI_DEV_CACHE nor HOME is set")?;
    Ok(PathBuf::from(home).join(".cache").join("nomi-dev"))
}

fn oracles_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("oracles"))
}

/// Where a binary oracle is installed, e.g. `oracles/elm-0.19.3/elm`.
pub fn binary_path(binary: &Binary) -> Result<PathBuf> {
    let exe = format!("{}{}", binary.name, std::env::consts::EXE_SUFFIX);
    Ok(oracles_dir()?
        .join(format!("{}-{}", binary.name, binary.version))
        .join(exe))
}

pub fn fetch(root: &Path, refs: &References) -> Result<()> {
    let platform = platform();
    for binary in &refs.binary {
        fetch_binary(binary, &platform)
            .with_context(|| format!("fetching {} {}", binary.name, binary.version))?;
    }
    node_version(refs.node_minimum)?;
    fetch_npm(root, refs).context("installing npm oracles")?;
    for package in &refs.elm_package {
        fetch_elm_package(package)
            .with_context(|| format!("fetching {} {}", package.name, package.version))?;
    }
    Ok(())
}

fn fetch_binary(binary: &Binary, platform: &str) -> Result<()> {
    let Some(asset) = binary.assets.get(platform) else {
        bail!("no asset is pinned for {platform}");
    };
    let path = binary_path(binary)?;
    let stamp = path.with_extension("sha256");
    if path.exists() && fs::read_to_string(&stamp).ok().as_deref() == Some(&asset.sha256) {
        eprintln!("  {} {} is installed", binary.name, binary.version);
        return Ok(());
    }
    let archive = download::get(&binary.url(asset))?;
    download::verify(&asset.file, &asset.sha256, &download::sha256(&archive))?;
    let exe = match binary.format {
        Format::Gz => download::gunzip(&archive)?,
        Format::Tgz => download::untgz_file(&archive, path.file_name().unwrap().to_str().unwrap())?,
    };
    download::write_atomic(&path, &exe)?;
    make_executable(&path)?;
    download::write_atomic(&stamp, asset.sha256.as_bytes())?;
    eprintln!(
        "  {} {} installed at {}",
        binary.name,
        binary.version,
        path.display()
    );
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// Installs `oracles/npm` with `npm ci --ignore-scripts`. The lockfile pins
/// every tarball's integrity; its hash is the install's stamp.
fn fetch_npm(root: &Path, refs: &References) -> Result<()> {
    let source = root.join("oracles").join("npm");
    let lock = fs::read_to_string(source.join("package-lock.json"))
        .context("reading oracles/npm/package-lock.json")?;
    check_lock_versions(&lock, refs)?;
    let dir = oracles_dir()?.join("npm");
    let stamp = dir.join(".lock-sha256");
    let hash = download::sha256(lock.as_bytes());
    if dir.join("node_modules").exists() && fs::read_to_string(&stamp).ok() == Some(hash.clone()) {
        eprintln!("  npm oracles are installed");
        return Ok(());
    }
    fs::create_dir_all(&dir)?;
    fs::copy(source.join("package.json"), dir.join("package.json"))?;
    fs::write(dir.join("package-lock.json"), &lock)?;
    eprintln!("  npm ci --ignore-scripts in {}", dir.display());
    let status = Command::new(npm())
        .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
        .current_dir(&dir)
        .status()
        .context("running npm; is Node installed?")?;
    if !status.success() {
        bail!("npm ci failed");
    }
    download::write_atomic(&stamp, hash.as_bytes())
}

fn npm() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

/// Each npm pin must be the version the lockfile installs.
pub fn check_lock_versions(lock: &str, refs: &References) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Lock {
        packages: std::collections::BTreeMap<String, Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        version: Option<String>,
    }
    let lock: Lock = serde_json::from_str(lock).context("parsing package-lock.json")?;
    for pin in &refs.npm {
        let key = format!("node_modules/{}", pin.name);
        let locked = lock.packages.get(&key).and_then(|e| e.version.as_deref());
        if locked != Some(pin.version.as_str()) {
            bail!(
                "references.toml pins {} {}, but package-lock.json has {}",
                pin.name,
                pin.version,
                locked.unwrap_or("nothing")
            );
        }
    }
    Ok(())
}

fn fetch_elm_package(package: &crate::references::ElmPackage) -> Result<()> {
    let path = cache_dir()?
        .join("elm-packages")
        .join(&package.name)
        .join(format!("{}.zip", package.version));
    if path.exists() && download::sha1(&fs::read(&path)?) == package.endpoint_hash {
        eprintln!("  {} {} is cached", package.name, package.version);
        return Ok(());
    }
    let zip = download::get(&format!(
        "https://github.com/{}/zipball/{}/",
        package.name, package.version
    ))?;
    download::verify(&package.name, &package.endpoint_hash, &download::sha1(&zip))?;
    download::write_atomic(&path, &zip)?;
    eprintln!("  {} {} cached", package.name, package.version);
    Ok(())
}

/// Node's major version, which must be at least `minimum`.
pub fn node_version(minimum: u32) -> Result<String> {
    let out = Command::new("node")
        .arg("--version")
        .output()
        .context("running node; the npm oracles need Node on PATH")?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let major: u32 = version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .with_context(|| format!("unrecognized node version {version:?}"))?;
    if major < minimum {
        bail!("node {version} is older than the minimum, v{minimum}");
    }
    Ok(version)
}

pub fn versions(refs: &References) -> Result<()> {
    println!("platform  {}", platform());
    match node_version(refs.node_minimum) {
        Ok(v) => println!("node      {v} (minimum v{})", refs.node_minimum),
        Err(e) => println!("node      {e:#}"),
    }
    println!();
    println!("sources (reference/)");
    for s in &refs.source {
        println!("  {:<34} {:<10} {}  {}", s.name, s.tag, s.commit, s.repo);
    }
    println!("binaries");
    for b in &refs.binary {
        let installed = binary_path(b)
            .map(|p| {
                if p.exists() {
                    "installed"
                } else {
                    "not fetched"
                }
            })
            .unwrap_or("not fetched");
        println!(
            "  {:<34} {:<10} {}  {installed}",
            b.name, b.version, b.commit
        );
    }
    println!("npm (oracles/npm/package-lock.json)");
    for n in &refs.npm {
        println!(
            "  {:<34} {:<10} {}  {}",
            n.name, n.version, n.commit, n.repo
        );
    }
    println!("elm packages");
    for p in &refs.elm_package {
        println!(
            "  {:<34} {:<10} {}  sha1 {}",
            p.name, p.version, p.commit, p.endpoint_hash
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::references::parse;

    fn refs() -> References {
        parse(concat!(
            "node-minimum = 22\n[[npm]]\nname = \"elm-test\"\nversion = \"0.19.3-0\"\n",
            "repo = \"a/b\"\ncommit = \"1c278bf5ecf4136f122ce1cba436783c7f561dac\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn matching_lock_passes() {
        let lock = r#"{"packages": {"": {}, "node_modules/elm-test": {"version": "0.19.3-0"}}}"#;
        assert!(check_lock_versions(lock, &refs()).is_ok());
    }

    #[test]
    fn drifted_lock_fails() {
        let lock = r#"{"packages": {"node_modules/elm-test": {"version": "0.19.2-1"}}}"#;
        let error = check_lock_versions(lock, &refs()).unwrap_err().to_string();
        assert!(error.contains("package-lock.json has 0.19.2-1"), "{error}");
    }

    #[test]
    fn repository_lock_matches_pins() {
        let refs = parse(include_str!("../../../references.toml")).unwrap();
        let lock = include_str!("../../../oracles/npm/package-lock.json");
        check_lock_versions(lock, &refs).unwrap();
    }
}
