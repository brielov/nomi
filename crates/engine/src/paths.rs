//! Every path nomi computes (rule R6): a project's files from its root,
//! and the per-user cache. Code elsewhere asks this module for a path
//! rather than joining names itself.
//!
//! nomi never writes upstream's files, `elm-stuff/0.19.x` and `~/.elm`
//! (`ELM_HOME`), so that elm and nomi can share a checkout and a home;
//! nothing here names them.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::Vfs;

/// The manifest of an Elm 0.19 project.
pub const ELM_JSON: &str = "elm.json";

/// The manifest of an Elm 0.18 project, which nomi only recognizes.
pub const ELM_PACKAGE_JSON: &str = "elm-package.json";

/// The variable naming the cache, like Go's `GOCACHE`.
pub const NOMI_CACHE: &str = "NOMI_CACHE";

/// A project: the directory holding its `elm.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root(PathBuf);

impl Root {
    /// The project around `dir`, which upstream takes from
    /// `getCurrentDirectory`: the nearest of `dir` and its ancestors,
    /// up to the file system's root, that holds a file `elm.json`, as
    /// `Stuff.findRoot` (0.19.1, 0.19.2) and `Root.findRoot` (0.19.3) walk.
    /// A directory named `elm.json` isn't one; a link to a file is.
    pub fn find(vfs: &Vfs, dir: &Path) -> Option<Root> {
        dir.ancestors()
            .find(|d| vfs.is_file(&d.join(ELM_JSON)))
            .map(|d| Root(d.to_owned()))
    }

    pub fn new(dir: PathBuf) -> Root {
        Root(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    pub fn elm_json(&self) -> PathBuf {
        self.0.join(ELM_JSON)
    }
}

/// nomi's one per-user cache (architecture §6): content-addressed, so
/// every project and every version of nomi share it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cache(PathBuf);

/// Why there is no cache directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheError {
    /// The variable names a relative path, which would move with the
    /// working directory.
    Relative(&'static str),
    /// No variable says where the user's caches go.
    Unknown,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Relative(var) => write!(f, "{var} must be an absolute path"),
            CacheError::Unknown => write!(
                f,
                "nomi can't tell where to keep its cache; set {NOMI_CACHE} to a directory"
            ),
        }
    }
}

impl std::error::Error for CacheError {}

impl Cache {
    /// `NOMI_CACHE`, else `nomi` in the user's cache directory.
    pub fn locate() -> Result<Cache, CacheError> {
        Cache::from_env(std::env::consts::OS, |var| std::env::var_os(var))
    }

    pub fn new(dir: PathBuf) -> Cache {
        Cache(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    /// The package index, shared by every release (`nomi_registry::Index`).
    pub fn index(&self) -> PathBuf {
        self.0.join("registry").join("index")
    }

    /// The `elm.json` of releases that aren't downloaded, which the solver
    /// reads (`nomi_registry::Registry`).
    pub fn metadata(&self) -> PathBuf {
        self.0.join("registry").join("metadata")
    }

    /// What builds made, by what they were made from
    /// (`artifacts::Artifacts`).
    pub fn artifacts(&self) -> PathBuf {
        self.0.join("artifacts")
    }

    /// The package store, shared by every release
    /// (`nomi_registry::Store`).
    pub fn packages(&self) -> PathBuf {
        self.0.join("packages")
    }

    /// [`Cache::locate`] on `os` with the variables `var` gives. The user's
    /// cache directory is Go's `os.UserCacheDir`: `%LocalAppData%` on
    /// Windows, `~/Library/Caches` on macOS, and elsewhere
    /// `$XDG_CACHE_HOME`, or `~/.cache` when that is unset or relative, as
    /// the XDG specification says.
    fn from_env(os: &str, var: impl Fn(&str) -> Option<OsString>) -> Result<Cache, CacheError> {
        let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        if let Some(dir) = set(NOMI_CACHE) {
            return if dir.is_absolute() {
                Ok(Cache(dir))
            } else {
                Err(CacheError::Relative(NOMI_CACHE))
            };
        }
        let home = || set("HOME").filter(|h| h.is_absolute());
        let user = match os {
            "windows" => set("LocalAppData").filter(|d| d.is_absolute()),
            "macos" | "ios" => home().map(|h| h.join("Library").join("Caches")),
            _ => set("XDG_CACHE_HOME")
                .filter(|d| d.is_absolute())
                .or_else(|| home().map(|h| h.join(".cache"))),
        };
        user.map(|dir| Cache(dir.join("nomi")))
            .ok_or(CacheError::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(os: &str, vars: &[(&str, &str)]) -> Result<Cache, CacheError> {
        Cache::from_env(os, |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        })
    }

    /// `path`, a Unix absolute path, made absolute on the machine running
    /// the tests: Windows wants a drive.
    fn abs(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{path}")
        } else {
            path.to_owned()
        }
    }

    fn at(dir: &str) -> Cache {
        Cache(PathBuf::from(abs(dir)))
    }

    #[test]
    fn the_variable_comes_first() {
        let (c, h) = (abs("/c"), abs("/h"));
        let vars = [(NOMI_CACHE, c.as_str()), ("HOME", h.as_str())];
        assert_eq!(cache("linux", &vars), Ok(at("/c")));
        assert_eq!(
            cache("linux", &[(NOMI_CACHE, "c")]),
            Err(CacheError::Relative(NOMI_CACHE))
        );
        assert_eq!(
            cache("linux", &[(NOMI_CACHE, ""), ("HOME", h.as_str())]),
            Ok(at("/h/.cache/nomi"))
        );
    }

    #[test]
    fn then_the_users_cache_directory() {
        let (h, x) = (abs("/h"), abs("/x"));
        let home = ("HOME", h.as_str());
        assert_eq!(cache("macos", &[home]), Ok(at("/h/Library/Caches/nomi")));
        assert_eq!(cache("linux", &[home]), Ok(at("/h/.cache/nomi")));
        assert_eq!(
            cache("freebsd", &[home, ("XDG_CACHE_HOME", x.as_str())]),
            Ok(at("/x/nomi"))
        );
        assert_eq!(
            cache("linux", &[home, ("XDG_CACHE_HOME", "x")]),
            Ok(at("/h/.cache/nomi"))
        );
        assert_eq!(cache("linux", &[]), Err(CacheError::Unknown));
        assert_eq!(cache("linux", &[("HOME", "h")]), Err(CacheError::Unknown));
    }
}
