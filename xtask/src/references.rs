//! `references.toml`: the pins every upstream download is checked against.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct References {
    pub node_minimum: u32,
    #[serde(default)]
    pub source: Vec<Source>,
    #[serde(default)]
    pub binary: Vec<Binary>,
    #[serde(default)]
    pub npm: Vec<Npm>,
    #[serde(default)]
    pub elm_package: Vec<ElmPackage>,
}

/// A source tree synced to `reference/<name>`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub name: String,
    pub repo: String,
    pub tag: String,
    pub commit: String,
}

/// A released executable, fetched from the repo's GitHub release `version`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binary {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub commit: String,
    pub format: Format,
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    /// A gzipped executable.
    Gz,
    /// A gzipped tarball whose only file is the executable.
    Tgz,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Asset {
    pub file: String,
    pub sha256: String,
}

/// A tool installed from npm; its integrity lives in the lockfile.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Npm {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub commit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ElmPackage {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub endpoint_hash: String,
}

impl Binary {
    pub(crate) fn url(&self, asset: &Asset) -> String {
        format!(
            "https://github.com/{}/releases/download/{}/{}",
            self.repo, self.version, asset.file
        )
    }
}

/// The current platform as `references.toml` spells it.
pub(crate) fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub(crate) fn load(root: &Path) -> Result<References> {
    let path = root.join("references.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text)
}

pub(crate) fn parse(text: &str) -> Result<References> {
    let refs: References = toml::from_str(text).context("parsing references.toml")?;
    validate(&refs)?;
    Ok(refs)
}

const PLATFORMS: &[&str] = &[
    "linux-x86_64",
    "linux-aarch64",
    "macos-x86_64",
    "macos-aarch64",
    "windows-x86_64",
];

fn validate(refs: &References) -> Result<()> {
    let hex = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit());
    let mut problems = Vec::new();
    let mut commit = |what: String, c: &str| {
        if !hex(c, 40) {
            problems.push(format!("{what}: commit must be 40 hex digits"));
        }
    };
    for s in &refs.source {
        commit(format!("source {}", s.name), &s.commit);
    }
    for b in &refs.binary {
        commit(format!("binary {} {}", b.name, b.version), &b.commit);
    }
    for n in &refs.npm {
        commit(format!("npm {}", n.name), &n.commit);
    }
    for p in &refs.elm_package {
        commit(format!("elm package {}", p.name), &p.commit);
    }
    for b in &refs.binary {
        for (platform, asset) in &b.assets {
            if !PLATFORMS.contains(&platform.as_str()) {
                problems.push(format!(
                    "binary {} {}: unknown platform {platform}",
                    b.name, b.version
                ));
            }
            if !hex(&asset.sha256, 64) {
                problems.push(format!(
                    "binary {} {}: sha256 must be 64 hex digits",
                    b.name, b.version
                ));
            }
        }
    }
    for p in &refs.elm_package {
        if !hex(&p.endpoint_hash, 40) {
            problems.push(format!(
                "elm package {}: endpoint-hash must be 40 hex digits",
                p.name
            ));
        }
    }
    let mut names: Vec<&str> = refs.source.iter().map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    if names.windows(2).any(|w| w[0] == w[1]) {
        problems.push("source names must be unique".to_string());
    }
    if problems.is_empty() {
        Ok(())
    } else {
        bail!("references.toml:\n  {}", problems.join("\n  "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "1c278bf5ecf4136f122ce1cba436783c7f561dac";

    #[test]
    fn repository_pins_are_valid() {
        let text = include_str!("../../references.toml");
        let refs = parse(text).unwrap();
        assert!(refs.node_minimum >= 22);
        assert!(refs.binary.iter().any(|b| b.name == "elm-format"));
    }

    #[test]
    fn short_commit_fails() {
        let text = "node-minimum = 22\n[[source]]\nname = \"c\"\nrepo = \"a/b\"\ntag = \"t\"\ncommit = \"1c278bf\"\n";
        let error = parse(text).unwrap_err().to_string();
        assert!(
            error.contains("source c: commit must be 40 hex digits"),
            "{error}"
        );
    }

    #[test]
    fn bad_asset_fails() {
        let text = format!(
            "node-minimum = 22\n[[binary]]\nname = \"elm\"\nversion = \"1\"\nrepo = \"a/b\"\ncommit = \"{COMMIT}\"\nformat = \"gz\"\nassets.plan9-mips = {{ file = \"f\", sha256 = \"abc\" }}\n"
        );
        let error = parse(&text).unwrap_err().to_string();
        assert!(error.contains("unknown platform plan9-mips"), "{error}");
        assert!(error.contains("sha256 must be 64 hex digits"), "{error}");
    }

    #[test]
    fn unknown_field_fails() {
        assert!(parse("node-minimum = 22\nextra = 1\n").is_err());
    }
}
