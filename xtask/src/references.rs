//! `references.toml`: the pins every upstream download is checked against.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Every pin.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct References {
    pub bun_minimum: String,
    pub node_minimum: u32,
    #[serde(default)]
    pub source: Vec<Source>,
    #[serde(default)]
    pub binary: Vec<Binary>,
    #[serde(default)]
    pub npm: Vec<Npm>,
    #[serde(default)]
    pub elm_package: Vec<ElmPackage>,
    #[serde(default)]
    pub image: Vec<Image>,
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
    pub assets: BTreeMap<Platform, Asset>,
}

/// A platform a pin has an asset for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) enum Platform {
    #[serde(rename = "linux-x86_64")]
    LinuxX86_64,
    #[serde(rename = "linux-aarch64")]
    LinuxAarch64,
    #[serde(rename = "macos-x86_64")]
    MacosX86_64,
    #[serde(rename = "macos-aarch64")]
    MacosAarch64,
    #[serde(rename = "windows-x86_64")]
    WindowsX86_64,
}

impl Platform {
    /// The platform xtask runs on, if it is one of them.
    pub(crate) fn current() -> Option<Platform> {
        use std::env::consts::{ARCH, OS};
        match (OS, ARCH) {
            ("linux", "x86_64") => Some(Platform::LinuxX86_64),
            ("linux", "aarch64") => Some(Platform::LinuxAarch64),
            ("macos", "x86_64") => Some(Platform::MacosX86_64),
            ("macos", "aarch64") => Some(Platform::MacosAarch64),
            ("windows", "x86_64") => Some(Platform::WindowsX86_64),
            _ => None,
        }
    }
}

/// How a binary's release asset holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    /// A gzipped executable.
    Gz,
    /// A gzipped tarball whose only file is the executable.
    Tgz,
}

/// A binary's release asset for one platform, and its hash.
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

/// A package release, pinned by the hash the package website gives its
/// GitHub zipball.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ElmPackage {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub endpoint_hash: String,
}

/// A container image a tier-2 service runs from, pinned by the digest of
/// its index.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Image {
    pub name: String,
    pub repository: String,
    pub tag: String,
    pub digest: String,
}

impl Image {
    /// The image by its digest alone, as it is pulled and run.
    pub(crate) fn reference(&self) -> String {
        format!("{}@{}", self.repository, self.digest)
    }
}

impl Binary {
    /// Where `asset` downloads from.
    pub(crate) fn url(&self, asset: &Asset) -> String {
        format!(
            "https://github.com/{}/releases/download/{}/{}",
            self.repo, self.version, asset.file
        )
    }
}

/// The current platform as `references.toml` spells one.
pub(crate) fn platform_name() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The pins in `root`'s `references.toml`.
pub(crate) fn load(root: &Path) -> Result<References> {
    let path = root.join("references.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text)
}

/// The pins `text` holds, checked.
pub(crate) fn parse(text: &str) -> Result<References> {
    let refs: References = toml::from_str(text).context("parsing references.toml")?;
    validate(&refs)?;
    Ok(refs)
}

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
        for asset in b.assets.values() {
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
    for i in &refs.image {
        if !i.digest.strip_prefix("sha256:").is_some_and(|d| hex(d, 64)) {
            problems.push(format!(
                "image {}: digest must be sha256: and 64 hex digits",
                i.name
            ));
        }
    }
    let unique = |what: &str, names: &mut Vec<&str>, problems: &mut Vec<String>| {
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            problems.push(format!("{what} names must be unique"));
        }
    };
    let mut names: Vec<&str> = refs.source.iter().map(|s| s.name.as_str()).collect();
    unique("source", &mut names, &mut problems);
    let mut names: Vec<&str> = refs.image.iter().map(|i| i.name.as_str()).collect();
    unique("image", &mut names, &mut problems);
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
        assert_eq!(refs.bun_minimum, "1.4");
        assert!(refs.binary.iter().any(|b| b.name == "elm-format"));
        let images: Vec<&str> = refs.image.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(images, ["postgres", "mysql", "valkey", "garage"]);
    }

    #[test]
    fn image_by_tag_fails() {
        let image = |digest: &str| {
            format!(
                "bun-minimum = \"1.4\"\nnode-minimum = 22\n[[image]]\nname = \"pg\"\nrepository = \"docker.io/library/postgres\"\ntag = \"18\"\ndigest = \"{digest}\"\n"
            )
        };
        let error = parse(&image("18-alpine")).unwrap_err().to_string();
        assert!(
            error.contains("image pg: digest must be sha256: and 64 hex digits"),
            "{error}"
        );
        let refs = parse(&image(&format!("sha256:{}", "0".repeat(64)))).unwrap();
        assert_eq!(
            refs.image[0].reference(),
            format!("docker.io/library/postgres@sha256:{}", "0".repeat(64))
        );
    }

    #[test]
    fn short_commit_fails() {
        let text = "bun-minimum = \"1.4\"\nnode-minimum = 22\n[[source]]\nname = \"c\"\nrepo = \"a/b\"\ntag = \"t\"\ncommit = \"1c278bf\"\n";
        let error = parse(text).unwrap_err().to_string();
        assert!(
            error.contains("source c: commit must be 40 hex digits"),
            "{error}"
        );
    }

    #[test]
    fn bad_asset_fails() {
        let asset = |platform: &str| {
            format!(
                "bun-minimum = \"1.4\"\nnode-minimum = 22\n[[binary]]\nname = \"elm\"\nversion = \"1\"\nrepo = \"a/b\"\ncommit = \"{COMMIT}\"\nformat = \"gz\"\nassets.{platform} = {{ file = \"f\", sha256 = \"abc\" }}\n"
            )
        };
        let error = format!("{:#}", parse(&asset("plan9-mips")).unwrap_err());
        assert!(error.contains("unknown variant `plan9-mips`"), "{error}");
        let error = parse(&asset("linux-x86_64")).unwrap_err().to_string();
        assert!(error.contains("sha256 must be 64 hex digits"), "{error}");
    }

    #[test]
    fn unknown_field_fails() {
        assert!(parse("bun-minimum = \"1.4\"\nnode-minimum = 22\nextra = 1\n").is_err());
    }
}
