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
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use nomi_base::Compat;
use nomi_project::json::Cursor;
use nomi_project::{Constraint, Version};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use super::{apps, corpus_dir, packages};
use crate::references;

const FILE: &str = "corpus/compat.toml";

/// The pinned elm releases, oldest first.
pub(crate) fn compats(refs: &references::References) -> Result<Vec<Compat>> {
    let mut compats = (refs.binary.iter())
        .filter(|b| b.name == "elm")
        .map(|b| Ok(b.version.parse()?))
        .collect::<Result<Vec<Compat>>>()?;
    compats.sort();
    Ok(compats)
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Classification {
    #[serde(deserialize_with = "compat_list")]
    pub compats: Vec<Compat>,
    /// Packages whose range leaves out a Compat; every other pinned
    /// package admits them all.
    #[serde(default)]
    package: Vec<Excluding>,
    #[serde(default)]
    app: Vec<AppPins>,
}

impl Classification {
    /// Whether `compat` builds the pinned package `name` at `version`.
    pub(crate) fn admits(&self, name: &str, version: &str, compat: Compat) -> bool {
        (self.package.iter())
            .find(|p| p.name == name && p.version == version)
            .is_none_or(|p| p.admits.contains(&compat))
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Excluding {
    name: String,
    version: String,
    /// Absent when the package has no `elm.json`, which no release can
    /// install.
    elm_version: Option<String>,
    #[serde(deserialize_with = "compat_list")]
    admits: Vec<Compat>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct AppPins {
    name: String,
    /// The hash of the app's `elm.json` rewritten for each Compat.
    #[serde(deserialize_with = "compat_keys")]
    manifest_sha256: BTreeMap<Compat, String>,
}

/// Compats, as their versions.
fn compat_list<'de, D: Deserializer<'de>>(compats: D) -> Result<Vec<Compat>, D::Error> {
    (Vec::<String>::deserialize(compats)?.iter())
        .map(|c| c.parse().map_err(D::Error::custom))
        .collect()
}

/// A map keyed by Compats, as their versions.
fn compat_keys<'de, D: Deserializer<'de>>(map: D) -> Result<BTreeMap<Compat, String>, D::Error> {
    (BTreeMap::<String, String>::deserialize(map)?.into_iter())
        .map(|(c, v)| Ok((c.parse().map_err(D::Error::custom)?, v)))
        .collect()
}

/// `corpus/compat.toml`.
pub(crate) fn load(root: &Path) -> Result<Classification> {
    let text = fs::read_to_string(root.join(FILE)).with_context(|| format!("reading {FILE}"))?;
    toml::from_str(&text).with_context(|| format!("parsing {FILE}"))
}

/// `elm.json`'s text with the value of its first `elm-version` set to
/// `compat`, every other byte kept; none if it has no such value.
fn with_elm_version(text: &str, compat: Compat) -> Option<String> {
    let key = "\"elm-version\"";
    let after = text.find(key)? + key.len();
    let open = after + text[after..].find('"')?;
    let close = open + 1 + text[open + 1..].find('"')?;
    Some(format!("{}{compat}{}", &text[..=open], &text[close..]))
}

/// An application's `elm.json` with its `elm-version` set to `compat`,
/// every other byte kept. Only applications name one version.
fn rewrite_manifest(text: &str, compat: Compat) -> Result<String> {
    let json: serde_json::Value = serde_json::from_str(text).context("parsing elm.json")?;
    ensure!(
        json["type"] == "application",
        "only an application's elm-version can be rewritten"
    );
    let rewritten = with_elm_version(text, compat).context("elm.json has no elm-version")?;
    let check: serde_json::Value = serde_json::from_str(&rewritten)?;
    ensure!(
        check["elm-version"] == compat.as_str(),
        "the elm-version rewrite missed"
    );
    Ok(rewritten)
}

/// An app's `elm.json` for a sandbox under `compat`, checked against the
/// pinned hash.
pub(crate) fn sandbox_manifest(
    classification: &Classification,
    app: &apps::App,
    compat: Compat,
) -> Result<String> {
    let text = apps::manifest(&corpus_dir()?, app)?;
    let rewritten = rewrite_manifest(&text, compat)?;
    let pin = classification
        .app
        .iter()
        .find(|a| a.name == app.name)
        .and_then(|a| a.manifest_sha256.get(&compat))
        .with_context(|| format!("{FILE} pins no {compat} manifest for {}", app.name))?;
    let actual = nomi_testkit::sha256_hex(rewritten.as_bytes());
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
        let path = packages::fetched_dir(&corpus, package)?.join("elm.json");
        let (elm_version, admits) = if path.exists() {
            let text =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let json: serde_json::Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}'s elm.json", package.name))?;
            let elm_version = json["elm-version"]
                .as_str()
                .with_context(|| format!("{} has no elm-version", package.name))?;
            let range = Constraint::parse(
                elm_version.as_bytes(),
                Cursor::start(Compat::LATEST),
                Compat::LATEST,
            )
            .ok()
            .with_context(|| format!("`{elm_version}` is not a version range"))?;
            let admits = (compats.iter().copied())
                .filter(|&c| range.satisfies(Version::compiler(c)))
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
        let text = apps::manifest(&corpus, &app)?;
        let manifest_sha256 = (compats.iter())
            .map(|&c| {
                let rewritten = rewrite_manifest(&text, c)?;
                Ok((c, nomi_testkit::sha256_hex(rewritten.as_bytes())))
            })
            .collect::<Result<_>>()?;
        app_pins.push(AppPins {
            name: app.name,
            manifest_sha256,
        });
    }
    Ok(render(&Classification {
        compats,
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
    let _ = writeln!(out, "compats = {:?}", names(&classification.compats));
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
        let _ = writeln!(out, "admits = {:?}", names(&p.admits));
    }
    for app in &classification.app {
        let _ = write!(out, "\n[[app]]\nname = {:?}\n", app.name);
        out.push_str("[app.manifest-sha256]\n");
        for (compat, hash) in &app.manifest_sha256 {
            let _ = writeln!(out, "{:?} = {hash:?}", compat.as_str());
        }
    }
    out
}

/// The versions of `compats`.
fn names(compats: &[Compat]) -> Vec<&'static str> {
    compats.iter().map(|c| c.as_str()).collect()
}

/// `cargo xtask corpus classify`: rewrites `corpus/compat.toml`.
pub(crate) fn classify(root: &Path) -> Result<()> {
    let text = classify_fetched(root)?;
    fs::write(root.join(FILE), text).with_context(|| format!("writing {FILE}"))?;
    println!("wrote {FILE}");
    Ok(())
}

/// The per-Compat lines of `cargo xtask corpus stats`.
pub(crate) fn stats(root: &Path) -> Result<()> {
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
                    sandbox_manifest(&classification, app, *compat)?;
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

    #[test]
    fn rewrites_keep_every_other_byte() {
        let text =
            "{\n    \"type\": \"application\",\n    \"elm-version\":\"0.19.1\",\n    \"x\": 1\n}\n";
        assert_eq!(
            rewrite_manifest(text, Compat::V0_19_3).unwrap(),
            text.replace("0.19.1", "0.19.3")
        );
    }

    #[test]
    fn the_first_elm_version_is_set_whatever_it_named() {
        let text = r#"{"elm-version": "0.18.0", "x": {"elm-version": "0.19.1"}}"#;
        assert_eq!(
            with_elm_version(text, Compat::V0_19_2).as_deref(),
            Some(r#"{"elm-version": "0.19.2", "x": {"elm-version": "0.19.1"}}"#)
        );
        assert_eq!(
            with_elm_version(r#"{"type": "application"}"#, Compat::V0_19_2),
            None
        );
        assert_eq!(
            with_elm_version(r#"{"elm-version": 1}"#, Compat::V0_19_2),
            None
        );
    }

    #[test]
    fn packages_are_not_rewritten() {
        let text = r#"{"type": "package", "elm-version": "0.19.0 <= v < 0.20.0"}"#;
        assert!(rewrite_manifest(text, Compat::V0_19_3).is_err());
    }

    #[test]
    fn rendered_classification_parses_back() {
        let classification = Classification {
            compats: vec![Compat::V0_19_1, Compat::V0_19_2],
            package: vec![Excluding {
                name: "a/b".into(),
                version: "1.0.0".into(),
                elm_version: Some("0.19.0 <= v < 0.19.2".into()),
                admits: vec![Compat::V0_19_1],
            }],
            app: vec![AppPins {
                name: "app".into(),
                manifest_sha256: [(Compat::V0_19_1, "ab".to_string())].into(),
            }],
        };
        let parsed: Classification = toml::from_str(&render(&classification)).unwrap();
        assert_eq!(parsed, classification);
        assert!(parsed.admits("a/b", "1.0.0", Compat::V0_19_1));
        assert!(!parsed.admits("a/b", "1.0.0", Compat::V0_19_2));
        assert!(parsed.admits("a/b", "2.0.0", Compat::V0_19_2));
    }
}
