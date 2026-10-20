//! `cargo xtask oracle`: the pinned upstream tools that parity is measured
//! against, installed under the development cache and never on `PATH`.

pub(crate) mod corpus_runs;
pub(crate) mod diff;
pub(crate) mod errors;
pub(crate) mod format;
pub(crate) mod javascript;
pub(crate) mod lsp;
pub(crate) mod lsp_import;
pub(crate) mod lsp_tables;
pub(crate) mod make;
pub(crate) mod members;
pub(crate) mod modules;
pub(crate) mod optimized;
pub(crate) mod outline;
pub(crate) mod process;
pub(crate) mod sandbox;
pub(crate) mod solver;
pub(crate) mod syntax;
pub(crate) mod transcripts;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use nomi_testkit::transcript::Case;

use crate::download;
use crate::references::{Binary, ElmPackage, Format, Platform, References, platform_name};

/// The packages the CLI's tests serve and the compiler's tests vendor,
/// but for the test runner's: what the oracles' projects may depend on.
const FIXTURE_PACKAGES: [&str; 10] = [
    "elm/browser",
    "elm/bytes",
    "elm/core",
    "elm/file",
    "elm/html",
    "elm/http",
    "elm/json",
    "elm/time",
    "elm/url",
    "elm/virtual-dom",
];

/// `$NOMI_DEV_CACHE`, or `~/.cache/nomi-dev`.
pub(crate) fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("NOMI_DEV_CACHE") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("neither NOMI_DEV_CACHE nor HOME is set")?;
    Ok(PathBuf::from(home).join(".cache").join("nomi-dev"))
}

/// Where the pinned tools are installed.
pub(crate) fn oracles_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("oracles"))
}

/// Where a binary oracle is installed, e.g. `oracles/elm-0.19.3/elm`.
pub(crate) fn binary_path(binary: &Binary) -> Result<PathBuf> {
    let exe = format!("{}{}", binary.name, std::env::consts::EXE_SUFFIX);
    Ok(oracles_dir()?
        .join(format!("{}-{}", binary.name, binary.version))
        .join(exe))
}

/// The pin of the binary `name`, at `version` where several are.
pub(crate) fn pin<'a>(
    refs: &'a References,
    name: &str,
    version: Option<&str>,
) -> Result<&'a Binary> {
    (refs.binary.iter())
        .find(|b| b.name == name && version.is_none_or(|v| b.version == v))
        .with_context(|| match version {
            Some(version) => format!("{name} {version} is not pinned"),
            None => format!("{name} is not pinned"),
        })
}

/// The installed binary pinned as `name`, at `version` where several are.
pub(crate) fn pinned_binary(
    refs: &References,
    name: &str,
    version: Option<&str>,
) -> Result<PathBuf> {
    installed(binary_path(pin(refs, name, version)?)?)
}

/// The installed elm of `compat`.
pub(crate) fn pinned_elm(refs: &References, compat: Compat) -> Result<PathBuf> {
    pinned_binary(refs, "elm", Some(compat.as_str()))
}

/// `path`, which `cargo xtask oracle fetch` installs, if it's there.
pub(crate) fn installed(path: PathBuf) -> Result<PathBuf> {
    ensure!(
        path.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        path.display()
    );
    Ok(path)
}

/// Node, and a pinned elm as `elm`, on a `PATH` of their own.
pub(crate) struct ElmOnPath {
    pub node: PathBuf,
    /// The pinned elm.
    pub elm: PathBuf,
    pub path: OsString,
}

/// Node, and `compat`'s pinned elm linked as `elm` in `bin`, on a `PATH`
/// of `bin`, Node's directory, then `extra`.
pub(crate) fn elm_on_path(
    refs: &References,
    compat: Compat,
    bin: &Path,
    extra: &[&Path],
) -> Result<ElmOnPath> {
    let elm = pinned_elm(refs, compat)?;
    let node = which("node").context("node isn't on PATH")?;
    fs::create_dir_all(bin)?;
    let linked = bin.join(elm.file_name().context("elm's name")?);
    if !linked.exists() {
        link(&elm, &linked)?;
    }
    let dirs = [bin, node.parent().context("node's directory")?];
    let path = std::env::join_paths(dirs.into_iter().chain(extra.iter().copied()))?;
    Ok(ElmOnPath { node, elm, path })
}

/// `from` at `to`, as a link where links are made without privileges,
/// and a copy elsewhere.
pub(crate) fn link(from: &Path, to: &Path) -> Result<()> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(from, to)?;
    #[cfg(not(unix))]
    if from.is_dir() {
        nomi_testkit::copy_tree(from, to, |_| true)?;
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// The first `name` on `PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The cores available, the default for how many runs go at once.
pub(crate) fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Removes the directory `dir` and everything in it, if it's there.
pub(crate) fn remove_tree(dir: &Path) -> Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// Makes `dir` an empty directory, whatever it held.
pub(crate) fn fresh_dir(dir: &Path) -> Result<()> {
    remove_tree(dir)?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))
}

/// The first line of a table `cargo xtask oracle <command>` writes.
pub(crate) fn generated(command: &str) -> String {
    format!("# @generated by `cargo xtask oracle {command}`; regenerate, never edit.")
}

/// A table's header line, without its newline: `first`, then a column
/// for each of `compats`.
fn elm_header(first: &str, compats: impl IntoIterator<Item = Compat>) -> String {
    let mut line = first.to_owned();
    for compat in compats {
        let _ = write!(line, "\telm {compat}");
    }
    line
}

/// The distinct outputs among each elm's, in the order first given, each
/// with the elms that gave it, as `elm 0.19.1`.
fn by_output<'a>(
    outputs: impl IntoIterator<Item = (Compat, &'a str)>,
) -> Vec<(Vec<String>, &'a str)> {
    let mut groups: Vec<(Vec<String>, &str)> = Vec::new();
    for (compat, output) in outputs {
        let elm = format!("elm {compat}");
        match groups.iter_mut().find(|(_, o)| *o == output) {
            Some((elms, _)) => elms.push(elm),
            None => groups.push((vec![elm], output)),
        }
    }
    groups
}

/// Each distinct transcript among each elm's, after a `### ` line naming
/// the elms that wrote it, as [`recorded`] reads them.
///
/// [`recorded`]: nomi_testkit::transcript::recorded
fn recordings<'a>(transcripts: impl IntoIterator<Item = (Compat, &'a str)>) -> String {
    let mut out = String::new();
    for (elms, transcript) in by_output(transcripts) {
        let _ = writeln!(out, "### {}", elms.join(", "));
        out.push_str(transcript);
    }
    out
}

/// Makes `work` afresh holding `case`'s files, laid out by
/// [`Case::lay_out_padded`] as the recordings were made and the tests
/// replay them; gives that directory.
fn padded_case(work: &Path, case: &Case) -> Result<PathBuf> {
    fresh_dir(work)?;
    Ok(case.lay_out_padded(work)?)
}

/// `dir` as text, for writing it `$ROOT` in what elm says.
fn utf8(dir: &Path) -> Result<&str> {
    dir.to_str().context("a scratch path that isn't UTF-8")
}

/// Downloads, verifies and installs every pinned oracle.
pub(crate) fn fetch(root: &Path, refs: &References) -> Result<()> {
    let platform = Platform::current();
    for binary in &refs.binary {
        fetch_binary(binary, platform)
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

fn fetch_binary(binary: &Binary, platform: Option<Platform>) -> Result<()> {
    let Some(asset) = platform.and_then(|p| binary.assets.get(&p)) else {
        bail!("no asset is pinned for {}", platform_name());
    };
    let path = binary_path(binary)?;
    let stamp = path.with_extension("sha256");
    if path.exists() && download::stamped(&stamp, &asset.sha256) {
        eprintln!("  {} {} is installed", binary.name, binary.version);
        return Ok(());
    }
    let archive = download::get(&binary.url(asset))?;
    download::verify(
        &asset.file,
        &asset.sha256,
        &nomi_testkit::sha256_hex(&archive),
    )?;
    let exe = match binary.format {
        Format::Gz => download::gunzip(&archive)?,
        Format::Tgz => {
            let name = path.file_name().expect("a binary's path names it");
            download::untgz_file(&archive, &name.to_string_lossy())?
        }
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

/// Installs `oracles/npm` with `npm ci --ignore-scripts`, or `nub ci` where
/// npm is missing, which installs from the same lockfile. The lockfile pins
/// every tarball's integrity; its hash is the install's stamp.
fn fetch_npm(root: &Path, refs: &References) -> Result<()> {
    let source = root.join("oracles").join("npm");
    let lock = fs::read_to_string(source.join("package-lock.json"))
        .context("reading oracles/npm/package-lock.json")?;
    check_lock_versions(&lock, refs)?;
    let dir = oracles_dir()?.join("npm");
    let stamp = dir.join(".lock-sha256");
    let hash = nomi_testkit::sha256_hex(lock.as_bytes());
    if dir.join("node_modules").exists() && download::stamped(&stamp, &hash) {
        eprintln!("  npm oracles are installed");
        return Ok(());
    }
    fs::create_dir_all(&dir)?;
    fs::copy(source.join("package.json"), dir.join("package.json"))?;
    fs::write(dir.join("package-lock.json"), &lock)?;
    eprintln!("  npm ci --ignore-scripts in {}", dir.display());
    let status = match Command::new(npm())
        .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
        .current_dir(&dir)
        .status()
    {
        // Without npm, nub installs strictly from the same lockfile.
        Err(e) if e.kind() == io::ErrorKind::NotFound => Command::new("nub")
            .args(["ci", "--ignore-scripts"])
            .current_dir(&dir)
            .status()
            .context("running npm or nub; is Node installed?")?,
        status => status.context("running npm; is Node installed?")?,
    };
    if !status.success() {
        bail!("npm ci failed");
    }
    download::write_atomic(&stamp, hash.as_bytes())
}

fn npm() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

/// Each npm pin must be the version the lockfile installs.
fn check_lock_versions(lock: &str, refs: &References) -> Result<()> {
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

fn fetch_elm_package(package: &ElmPackage) -> Result<()> {
    let path = cache_dir()?
        .join("elm-packages")
        .join(&package.name)
        .join(format!("{}.zip", package.version));
    if path.exists() && nomi_registry::store::sha1(&fs::read(&path)?) == package.endpoint_hash {
        eprintln!("  {} {} is cached", package.name, package.version);
        return Ok(());
    }
    let url = download::zipball(&package.name, &package.version);
    let zip = download::get(&url)?;
    download::verify(
        &package.name,
        &package.endpoint_hash,
        &nomi_registry::store::sha1(&zip),
    )?;
    download::write_atomic(&path, &zip)?;
    eprintln!("  {} {} cached", package.name, package.version);
    Ok(())
}

/// Node's version, as `node --version` prints it, failing unless its
/// major is at least `minimum`.
/// Bun's version, failing unless it is at least `minimum`, a version's
/// leading parts.
pub(crate) fn bun_version(minimum: &str) -> Result<String> {
    let out = Command::new("bun")
        .arg("--version")
        .output()
        .context("running bun; nomi's programs and tests need Bun on PATH")?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parts = |text: &str| -> Option<Vec<u32>> {
        text.split('.').map(|part| part.parse().ok()).collect()
    };
    let have = parts(&version).with_context(|| format!("unrecognized bun version {version:?}"))?;
    let need = parts(minimum).with_context(|| format!("unrecognized bun-minimum {minimum:?}"))?;
    if have[..need.len().min(have.len())] < need[..] {
        bail!("bun {version} is older than the minimum, {minimum}");
    }
    Ok(version)
}

pub(crate) fn node_version(minimum: u32) -> Result<String> {
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

/// Prints every pin and whether it is installed.
pub(crate) fn versions(refs: &References) {
    println!("platform  {}", platform_name());
    match bun_version(&refs.bun_minimum) {
        Ok(v) => println!("bun       {v} (minimum {})", refs.bun_minimum),
        Err(e) => println!("bun       {e:#}"),
    }
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
        let installed = if binary_path(b).is_ok_and(|p| p.exists()) {
            "installed"
        } else {
            "not fetched"
        };
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
}

#[cfg(test)]
mod tests {
    use nomi_testkit::transcript::recorded;

    use super::*;
    use crate::references::parse;

    fn refs() -> References {
        parse(concat!(
            "bun-minimum = \"1.4\"\nnode-minimum = 22\n[[npm]]\nname = \"elm-test\"\nversion = \"0.19.3-0\"\n",
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
    fn unpinned_and_unfetched_binaries_say_so() {
        let error = pinned_binary(&refs(), "elm", Some("0.18.0")).unwrap_err();
        assert_eq!(error.to_string(), "elm 0.18.0 is not pinned");
        let error = pinned_binary(&refs(), "elm-format", None).unwrap_err();
        assert_eq!(error.to_string(), "elm-format is not pinned");
        let error = installed(PathBuf::from("/nowhere/elm")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "/nowhere/elm is missing; run `cargo xtask oracle fetch`"
        );
    }

    #[test]
    fn recordings_name_the_elms_of_each_transcript() {
        let text = recordings([
            (Compat::V0_19_1, "a\n"),
            (Compat::V0_19_2, "b\n"),
            (Compat::V0_19_3, "a\n"),
        ]);
        assert_eq!(text, "### elm 0.19.1, elm 0.19.3\na\n### elm 0.19.2\nb\n");
        for (compat, transcript) in [(Compat::V0_19_3, "a\n"), (Compat::V0_19_2, "b\n")] {
            assert_eq!(recorded(&text, compat), Some(transcript));
        }
    }

    #[test]
    fn repository_lock_matches_pins() {
        let refs = parse(include_str!("../../../references.toml")).unwrap();
        let lock = include_str!("../../../oracles/npm/package-lock.json");
        check_lock_versions(lock, &refs).unwrap();
    }
}
