//! Real applications, pinned by commit and tree, checked out into the cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::{corpus_dir, files_below, tree_digest};
use crate::download;
use crate::references::platform;
use crate::refs;

const FILE: &str = "corpus/apps.toml";
/// Written into a checkout's `.git` once its preparation output has been
/// verified; there it never makes the work tree look changed.
const PREPARED: &str = ".git/nomi-prepared";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Apps {
    pub app: Vec<App>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct App {
    pub name: String,
    pub git: String,
    pub commit: String,
    pub tree: String,
    pub root: String,
    pub entries: Vec<String>,
    pub manifest_sha256: String,
    pub prepare: Option<Prepare>,
}

/// Code generation with sbt that must run before the app compiles.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Prepare {
    pub sbt_task: String,
    pub input: String,
    pub input_sha256: String,
    pub output: String,
    pub files: usize,
    pub output_sha256: String,
    pub sbt_launcher: Pinned,
    pub jdk: BTreeMap<String, Pinned>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pinned {
    pub url: String,
    pub sha256: String,
}

pub(crate) fn load(root: &Path) -> Result<Apps> {
    let text =
        std::fs::read_to_string(root.join(FILE)).with_context(|| format!("reading {FILE}"))?;
    toml::from_str(&text).with_context(|| format!("parsing {FILE}"))
}

pub(crate) fn app_dir(corpus: &Path, app: &App) -> PathBuf {
    corpus.join("apps").join(&app.name)
}

pub(crate) fn fetch(root: &Path) -> Result<()> {
    let corpus = corpus_dir()?;
    for app in load(root)?.app {
        fetch_one(&corpus, &app).with_context(|| format!("app {}", app.name))?;
    }
    Ok(())
}

fn fetch_one(corpus: &Path, app: &App) -> Result<()> {
    let dir = app_dir(corpus, app);
    refs::checkout(&dir, &app.git, &app.commit, &format!("app {}", app.name))?;
    let tree = refs::tree(&dir)?;
    download::verify("tree", &app.tree, &tree)?;
    let root = dir.join(&app.root);
    let manifest = std::fs::read(root.join("elm.json")).context("reading elm.json")?;
    download::verify(
        "elm.json",
        &app.manifest_sha256,
        &download::sha256(&manifest),
    )?;
    for entry in &app.entries {
        if !root.join(entry).is_file() {
            bail!("entry {entry} is missing");
        }
    }
    if let Some(prepare) = &app.prepare {
        run_prepare(corpus, &dir, prepare).context("preparing")?;
    }
    Ok(())
}

fn run_prepare(corpus: &Path, dir: &Path, prepare: &Prepare) -> Result<()> {
    let input = std::fs::read(dir.join(&prepare.input)).context("reading the input")?;
    download::verify(
        &prepare.input,
        &prepare.input_sha256,
        &download::sha256(&input),
    )?;
    let output = dir.join(&prepare.output);
    let stamp = dir.join(PREPARED);
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(prepare.output_sha256.as_str()) {
        let java = jdk(corpus, prepare)?;
        let launcher = cached(corpus, &prepare.sbt_launcher, "sbt-launch.jar")?;
        let sbt = corpus.join("tools").join("sbt-home");
        eprintln!(
            "  running sbt {} (first run downloads sbt's own dependencies)",
            prepare.sbt_task
        );
        let status = Command::new(&java)
            .current_dir(dir)
            .env("COURSIER_CACHE", sbt.join("coursier"))
            .arg(format!(
                "-Dsbt.global.base={}",
                sbt.join("global").display()
            ))
            .arg(format!(
                "-Dsbt.boot.directory={}",
                sbt.join("boot").display()
            ))
            .arg(format!("-Dsbt.ivy.home={}", sbt.join("ivy").display()))
            .arg("-jar")
            .arg(&launcher)
            .arg(&prepare.sbt_task)
            .status()
            .context("running java")?;
        if !status.success() {
            bail!("sbt {} failed", prepare.sbt_task);
        }
    }
    let files = files_below(&output)?;
    if files.len() != prepare.files {
        bail!(
            "expected {} generated files, found {}",
            prepare.files,
            files.len()
        );
    }
    download::verify(
        &prepare.output,
        &prepare.output_sha256,
        &tree_digest(&output, &files)?,
    )?;
    std::fs::write(&stamp, &prepare.output_sha256)?;
    eprintln!("  {} verified ({} files)", prepare.output, files.len());
    Ok(())
}

/// Downloads `pinned` into the tool cache once and returns its path.
fn cached(corpus: &Path, pinned: &Pinned, name: &str) -> Result<PathBuf> {
    let path = corpus.join("tools").join(name);
    if path.exists() && download::sha256(&std::fs::read(&path)?) == pinned.sha256 {
        return Ok(path);
    }
    let bytes = download::get(&pinned.url)?;
    download::verify(name, &pinned.sha256, &download::sha256(&bytes))?;
    download::write_atomic(&path, &bytes)?;
    Ok(path)
}

/// The pinned JDK's `java`, unpacked once into the tool cache.
fn jdk(corpus: &Path, prepare: &Prepare) -> Result<PathBuf> {
    let platform = platform();
    let Some(pinned) = prepare.jdk.get(&platform) else {
        bail!("no JDK is pinned for {platform}");
    };
    let dir = corpus
        .join("tools")
        .join(format!("jdk-{}", &pinned.sha256[..12]));
    if let Some(java) = find_java(&dir) {
        return Ok(java);
    }
    let archive = cached(corpus, pinned, "jdk.tar.gz")?;
    let bytes = std::fs::read(&archive)?;
    let partial = dir.with_extension("partial");
    if partial.exists() {
        std::fs::remove_dir_all(&partial)?;
    }
    tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]))
        .unpack(&partial)
        .context("unpacking the JDK")?;
    std::fs::rename(&partial, &dir)?;
    find_java(&dir).context("the JDK archive has no bin/java")
}

fn find_java(dir: &Path) -> Option<PathBuf> {
    let top = std::fs::read_dir(dir).ok()?.flatten().next()?.path();
    let exe = format!("java{}", std::env::consts::EXE_SUFFIX);
    [
        top.join("bin").join(&exe),
        top.join("Contents/Home/bin").join(&exe),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_apps_parse() {
        let apps: Apps = toml::from_str(include_str!("../../../corpus/apps.toml")).unwrap();
        let names: Vec<&str> = apps.app.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names[..4],
            ["exosphere", "liascript", "docspell", "elm-3d-scene"]
        );
        // The apps added for app-shaped code, each once.
        assert_eq!(names.len(), 38);
        let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len());
        let prepare = apps.app[2].prepare.as_ref().unwrap();
        assert!(prepare.jdk.contains_key("macos-aarch64"));
    }

    #[test]
    fn unknown_field_fails() {
        let text = "[[app]]\nname = \"a\"\ngit = \"g\"\ncommit = \"c\"\ntree = \"t\"\nroot = \".\"\nentries = []\nmanifest-sha256 = \"m\"\nextra = 1\n";
        assert!(toml::from_str::<Apps>(text).is_err());
    }
}
