//! Which Compat accepts which corpus project. The Compats are the elm
//! releases pinned in `references.toml`. Upstream builds an application
//! only when its `elm-version` equals the compiler's (`Elm/Details.hs`,
//! `verifyApp`), and a package only when its range admits the compiler.
//!
//! Oracle sandboxes therefore rewrite each app's `elm-version` to the
//! Compat under test, and `corpus/compat.toml` pins each rewrite's hash.
//! A package whose range leaves a Compat out is that Compat's expected
//! failure, not a gap.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use super::{apps, corpus_dir, packages};
use crate::{download, references};

const FILE: &str = "corpus/compat.toml";

/// An elm release, such as 0.19.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version([u32; 3]);

impl Version {
    pub fn parse(text: &str) -> Result<Version> {
        let parts: Vec<&str> = text.split('.').collect();
        let [major, minor, patch] = parts[..] else {
            bail!("`{text}` is not a version");
        };
        let number = |s: &str| {
            s.parse::<u32>()
                .with_context(|| format!("`{text}` is not a version"))
        };
        Ok(Version([number(major)?, number(minor)?, number(patch)?]))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.0;
        write!(f, "{major}.{minor}.{patch}")
    }
}

/// A package's `elm-version`, such as `0.19.0 <= v < 0.20.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    lower: Version,
    lower_inclusive: bool,
    upper: Version,
    upper_inclusive: bool,
}

impl Range {
    pub fn parse(text: &str) -> Result<Range> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let [lower, lower_op, "v", upper_op, upper] = words[..] else {
            bail!("`{text}` is not a version range");
        };
        let inclusive = |op: &str| match op {
            "<" => Ok(false),
            "<=" => Ok(true),
            _ => bail!("`{text}` is not a version range"),
        };
        Ok(Range {
            lower: Version::parse(lower)?,
            lower_inclusive: inclusive(lower_op)?,
            upper: Version::parse(upper)?,
            upper_inclusive: inclusive(upper_op)?,
        })
    }

    pub fn admits(self, version: Version) -> bool {
        let above = if self.lower_inclusive {
            self.lower <= version
        } else {
            self.lower < version
        };
        let below = if self.upper_inclusive {
            version <= self.upper
        } else {
            version < self.upper
        };
        above && below
    }
}

/// The pinned elm releases, oldest first.
pub fn compats(refs: &references::References) -> Result<Vec<Version>> {
    let mut versions = refs
        .binary
        .iter()
        .filter(|b| b.name == "elm")
        .map(|b| Version::parse(&b.version))
        .collect::<Result<Vec<_>>>()?;
    versions.sort();
    Ok(versions)
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Classification {
    pub compats: Vec<String>,
    /// Packages whose range leaves out a Compat; every other pinned
    /// package admits them all.
    #[serde(default)]
    pub package: Vec<Excluding>,
    #[serde(default)]
    pub app: Vec<AppPins>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Excluding {
    pub name: String,
    pub version: String,
    /// Absent when the package has no `elm.json`, which no release can
    /// install.
    pub elm_version: Option<String>,
    pub admits: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AppPins {
    pub name: String,
    /// The hash of the app's `elm.json` rewritten for each Compat.
    pub manifest_sha256: BTreeMap<String, String>,
}

pub fn load(root: &Path) -> Result<Classification> {
    let text =
        std::fs::read_to_string(root.join(FILE)).with_context(|| format!("reading {FILE}"))?;
    toml::from_str(&text).with_context(|| format!("parsing {FILE}"))
}

/// `elm.json` with its `elm-version` set to `compat`, every other byte
/// kept. Only applications name one version.
pub fn rewrite_manifest(text: &str, compat: Version) -> Result<String> {
    let json: serde_json::Value = serde_json::from_str(text).context("parsing elm.json")?;
    ensure!(
        json["type"] == "application",
        "only an application's elm-version can be rewritten"
    );
    let key = "\"elm-version\"";
    let at = text.find(key).context("elm.json has no elm-version")?;
    let rest = &text[at + key.len()..];
    let open = at + key.len() + rest.find('"').context("elm-version has no value")?;
    let close = open
        + 1
        + text[open + 1..]
            .find('"')
            .context("unterminated elm-version")?;
    let rewritten = format!("{}{compat}{}", &text[..=open], &text[close..]);
    let check: serde_json::Value = serde_json::from_str(&rewritten)?;
    ensure!(
        check["elm-version"] == compat.to_string().as_str(),
        "the elm-version rewrite missed"
    );
    Ok(rewritten)
}

/// An app's `elm.json` for a sandbox under `compat`, checked against the
/// pinned hash.
pub fn sandbox_manifest(
    classification: &Classification,
    app: &apps::App,
    compat: Version,
) -> Result<String> {
    let path = apps::app_dir(&corpus_dir()?, app)
        .join(&app.root)
        .join("elm.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}; run `cargo xtask corpus fetch`", path.display()))?;
    let rewritten = rewrite_manifest(&text, compat)?;
    let pin = classification
        .app
        .iter()
        .find(|a| a.name == app.name)
        .and_then(|a| a.manifest_sha256.get(&compat.to_string()))
        .with_context(|| format!("{FILE} pins no {compat} manifest for {}", app.name))?;
    let actual = download::sha256(rewritten.as_bytes());
    ensure!(
        &actual == pin,
        "{}'s {compat} manifest hashes to {actual}, not {pin}",
        app.name
    );
    Ok(rewritten)
}

/// Classifies the fetched corpus afresh.
fn classify_fetched(root: &Path) -> Result<String> {
    let refs = references::load(root)?;
    let compats = compats(&refs)?;
    let corpus = corpus_dir()?;
    let mut excluding = Vec::new();
    for package in &packages::load(root)?.packages {
        let dir = packages::package_dir(&corpus, package);
        ensure!(
            dir.join(packages::STAMP).exists(),
            "{} {} is not fetched; run `cargo xtask corpus fetch`",
            package.name,
            package.version
        );
        let path = dir.join("elm.json");
        let (elm_version, admits) = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let json: serde_json::Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}'s elm.json", package.name))?;
            let elm_version = json["elm-version"]
                .as_str()
                .with_context(|| format!("{} has no elm-version", package.name))?;
            let range = Range::parse(elm_version)?;
            let admits = (compats.iter())
                .filter(|&&c| range.admits(c))
                .map(Version::to_string)
                .collect();
            (Some(elm_version.to_owned()), admits)
        } else {
            (None, Vec::new())
        };
        if admits.len() < compats.len() {
            excluding.push(Excluding {
                name: package.name.clone(),
                version: package.version.clone(),
                elm_version,
                admits,
            });
        }
    }
    let mut app_pins = Vec::new();
    for app in apps::load(root)?.app {
        let path = apps::app_dir(&corpus, &app)
            .join(&app.root)
            .join("elm.json");
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!("reading {}; run `cargo xtask corpus fetch`", path.display())
        })?;
        let manifest_sha256 = (compats.iter())
            .map(|&c| {
                Ok((
                    c.to_string(),
                    download::sha256(rewrite_manifest(&text, c)?.as_bytes()),
                ))
            })
            .collect::<Result<_>>()?;
        app_pins.push(AppPins {
            name: app.name,
            manifest_sha256,
        });
    }
    Ok(render(&Classification {
        compats: compats.iter().map(Version::to_string).collect(),
        package: excluding,
        app: app_pins,
    }))
}

fn render(classification: &Classification) -> String {
    let mut out = String::from(
        "# @generated by `cargo xtask corpus classify` from the fetched corpus;\n\
         # edit by re-classifying. The Compats are the elm releases pinned in\n\
         # references.toml. Every pinned package admits all of them except the\n\
         # ones listed, and each app's elm.json is rewritten to name the Compat\n\
         # under test, pinned here by hash.\n\n",
    );
    let _ = writeln!(out, "compats = {:?}", classification.compats);
    for p in &classification.package {
        let _ = write!(
            out,
            "\n[[package]]\nname = {:?}\nversion = {:?}\n",
            p.name, p.version
        );
        match &p.elm_version {
            Some(range) => {
                let _ = writeln!(out, "elm-version = {range:?}");
            }
            None => out.push_str("# no elm.json\n"),
        }
        let _ = writeln!(out, "admits = {:?}", p.admits);
    }
    for app in &classification.app {
        let _ = write!(out, "\n[[app]]\nname = {:?}\n", app.name);
        out.push_str("[app.manifest-sha256]\n");
        for (compat, hash) in &app.manifest_sha256 {
            let _ = writeln!(out, "{compat:?} = {hash:?}");
        }
    }
    out
}

/// `cargo xtask corpus classify`: rewrites `corpus/compat.toml`.
pub fn classify(root: &Path) -> Result<()> {
    let text = classify_fetched(root)?;
    std::fs::write(root.join(FILE), text).with_context(|| format!("writing {FILE}"))?;
    println!("wrote {FILE}");
    Ok(())
}

/// The per-Compat lines of `cargo xtask corpus stats`.
pub fn stats(root: &Path) -> Result<()> {
    let classification = load(root)?;
    let pinned = packages::load(root)?.packages.len();
    let apps = apps::load(root)?.app;
    for compat in &classification.compats {
        let excluded = (classification.package.iter())
            .filter(|p| !p.admits.contains(compat))
            .count();
        println!(
            "compat    {compat:<14} {} packages accepted, {excluded} expected failures, {} apps",
            pinned - excluded,
            apps.len()
        );
    }
    match classify_fetched(root) {
        Ok(fresh) if fresh == render(&classification) => {
            for app in &apps {
                for compat in &classification.compats {
                    sandbox_manifest(&classification, app, Version::parse(compat)?)?;
                }
            }
            println!("compat    {FILE} matches the fetched corpus");
        }
        Ok(_) => println!("compat    {FILE} is stale; run `cargo xtask corpus classify`"),
        Err(error) => println!("compat    not checked: {error:#}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn ranges_admit_by_their_operators() {
        let range = Range::parse("0.19.0 <= v < 0.19.2").unwrap();
        assert!(range.admits(v("0.19.0")));
        assert!(range.admits(v("0.19.1")));
        assert!(!range.admits(v("0.19.2")));
        let range = Range::parse("0.17.1 <= v <= 0.19.0").unwrap();
        assert!(range.admits(v("0.19.0")));
        assert!(!range.admits(v("0.19.1")));
        let range = Range::parse("0.19.1 < v <= 0.19.3").unwrap();
        assert!(!range.admits(v("0.19.1")));
        assert!(range.admits(v("0.19.3")));
    }

    #[test]
    fn malformed_ranges_are_errors() {
        for text in [
            "0.19.0 <= v",
            "0.19 <= v < 0.20.0",
            "0.19.0 >= v < 0.20.0",
            "",
        ] {
            assert!(Range::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn versions_order_numerically() {
        assert!(v("0.19.10") > v("0.19.9"));
        assert_eq!(v("0.19.1").to_string(), "0.19.1");
    }

    #[test]
    fn rewrites_keep_every_other_byte() {
        let text =
            "{\n    \"type\": \"application\",\n    \"elm-version\":\"0.19.1\",\n    \"x\": 1\n}\n";
        assert_eq!(
            rewrite_manifest(text, v("0.19.3")).unwrap(),
            text.replace("0.19.1", "0.19.3")
        );
    }

    #[test]
    fn packages_are_not_rewritten() {
        let text = r#"{"type": "package", "elm-version": "0.19.0 <= v < 0.20.0"}"#;
        assert!(rewrite_manifest(text, v("0.19.3")).is_err());
    }

    #[test]
    fn rendered_classification_parses_back() {
        let classification = Classification {
            compats: vec!["0.19.1".into(), "0.19.2".into()],
            package: vec![Excluding {
                name: "a/b".into(),
                version: "1.0.0".into(),
                elm_version: Some("0.19.0 <= v < 0.19.2".into()),
                admits: vec!["0.19.1".into()],
            }],
            app: vec![AppPins {
                name: "app".into(),
                manifest_sha256: [("0.19.1".to_string(), "ab".to_string())].into(),
            }],
        };
        let parsed: Classification = toml::from_str(&render(&classification)).unwrap();
        assert_eq!(parsed, classification);
    }
}
