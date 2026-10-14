//! The solver scenarios under `crates/registry/tests/solver/`, which
//! `cargo xtask oracle solver` answers with upstream's solvers and
//! registry's tests replay: a registry, the metadata of its releases, a
//! project, and perhaps a package to add.

use std::collections::BTreeMap;

use nomi_base::Compat;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// A solver scenario, as `scenarios/<name>.json` and each entry of
/// `generated.json` hold it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// `universes/<name>.json`, whose registry and releases come first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub universe: Option<String>,
    /// Each package's versions, newest first.
    #[serde(default)]
    pub registry: BTreeMap<String, Vec<String>>,
    /// Each release's metadata, by `<name>@<version>`. A release the
    /// registry lists without metadata isn't in the package cache.
    #[serde(default)]
    pub releases: BTreeMap<String, Release>,
    pub project: Project,
    /// The package to add, as `install` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<String>,
}

/// A registry and the metadata of its releases.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Universe {
    pub registry: BTreeMap<String, Vec<String>>,
    pub releases: BTreeMap<String, Release>,
}

impl Universe {
    /// The registry and releases `scenario` sees: its own over these.
    pub fn with(mut self, scenario: &Scenario) -> Universe {
        self.registry.extend(scenario.registry.clone());
        self.releases.extend(scenario.releases.clone());
        self
    }
}

/// A release's metadata.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Its `elm-version`.
    pub elm: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub deps: BTreeMap<String, String>,
    /// Whether its sources are in the cache; offline, upstream skips a
    /// release without them.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub src: bool,
    /// Whether its `elm.json` is unreadable.
    #[serde(default, skip_serializing_if = "is_no")]
    pub broken: bool,
}

impl Release {
    /// Its `elm.json`, as `name` at `version`: a package exposing
    /// `Fixture`, or `{}` if it is broken.
    pub fn elm_json(&self, name: &str, version: &str) -> String {
        if self.broken {
            return "{}".to_owned();
        }
        json!({
            "type": "package",
            "name": name,
            "summary": "fixture",
            "license": "BSD-3-Clause",
            "version": version,
            "exposed-modules": ["Fixture"],
            "elm-version": self.elm,
            "dependencies": self.deps,
            "test-dependencies": {},
        })
        .to_string()
    }
}

/// The project to solve for.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Project {
    #[serde(rename_all = "kebab-case")]
    Application {
        /// Its `elm-version`; the Compat's own by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elm: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        direct: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        indirect: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        test_direct: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        test_indirect: BTreeMap<String, String>,
    },
    #[serde(rename_all = "kebab-case")]
    Package {
        elm: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        deps: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        test_deps: BTreeMap<String, String>,
    },
}

impl Default for Project {
    fn default() -> Project {
        Project::Package {
            elm: String::new(),
            deps: BTreeMap::new(),
            test_deps: BTreeMap::new(),
        }
    }
}

impl Project {
    /// Its `elm.json`, as `compat`'s elm reads it.
    pub fn elm_json(&self, compat: Compat) -> serde_json::Value {
        match self {
            Project::Application {
                elm,
                direct,
                indirect,
                test_direct,
                test_indirect,
            } => json!({
                "type": "application",
                "source-directories": ["src"],
                "elm-version": elm.as_deref().unwrap_or(compat.as_str()),
                "dependencies": { "direct": direct, "indirect": indirect },
                "test-dependencies": { "direct": test_direct, "indirect": test_indirect },
            }),
            Project::Package {
                elm,
                deps,
                test_deps,
            } => json!({
                "type": "package",
                "name": "nomi/root",
                "summary": "fixture",
                "license": "BSD-3-Clause",
                "version": "1.0.0",
                "exposed-modules": ["Main"],
                "elm-version": elm,
                "dependencies": deps,
                "test-dependencies": test_deps,
            }),
        }
    }
}

fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_yes(b: &bool) -> bool {
    *b
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_no(b: &bool) -> bool {
    !*b
}
