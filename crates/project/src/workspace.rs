//! A workspace: its member patterns, paths relative to the root
//! `nomi.toml` where `*` stands for any one directory, and the graph of
//! the members found for them. Pure: the engine lists the directories and
//! reads each member's files.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{Manifest, Name, Outline};

/// A member pattern from `[workspace] members`, kept as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    text: String,
    segments: Box<[Segment]>,
}

/// One component of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Name(String),
    /// `*`: any one component not starting with `.`.
    Any,
}

impl Pattern {
    /// The pattern `text`, unless it is empty, absolute, has `..` or uses
    /// `*` in part of a component.
    pub fn parse(text: &str) -> Option<Pattern> {
        let mut segments = Vec::new();
        for component in Path::new(text).components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => {
                    let name = name.to_str()?;
                    segments.push(match name {
                        "*" => Segment::Any,
                        _ if name.contains('*') => return None,
                        _ => Segment::Name(name.to_owned()),
                    });
                }
                Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
            }
        }
        (!segments.is_empty()).then(|| Pattern {
            text: text.to_owned(),
            segments: segments.into(),
        })
    }

    /// Whether `path`, relative to the root, is a directory this pattern
    /// names.
    pub fn matches(&self, path: &Path) -> bool {
        let mut names = path.components().filter(|c| *c != Component::CurDir);
        self.segments.iter().all(|segment| match names.next() {
            Some(Component::Normal(name)) => segment.matches(name),
            _ => false,
        }) && names.next().is_none()
    }

    /// Whether the pattern names one directory, having no `*`.
    pub fn is_literal(&self) -> bool {
        !self.segments.contains(&Segment::Any)
    }

    /// The directories, relative to the root, that this pattern names,
    /// where `subdirs` gives the names of the directories in one.
    pub fn expand(
        &self,
        mut subdirs: impl FnMut(&Path) -> io::Result<Vec<OsString>>,
    ) -> io::Result<Vec<PathBuf>> {
        let mut dirs = vec![PathBuf::new()];
        for segment in &self.segments {
            dirs = match segment {
                Segment::Name(name) => dirs.into_iter().map(|dir| dir.join(name)).collect(),
                Segment::Any => {
                    let mut found = Vec::new();
                    for dir in &dirs {
                        for name in subdirs(dir)? {
                            if segment.matches(&name) {
                                found.push(dir.join(name));
                            }
                        }
                    }
                    found
                }
            };
        }
        Ok(dirs)
    }
}

impl Segment {
    fn matches(&self, name: &OsStr) -> bool {
        match self {
            Segment::Name(literal) => name == literal.as_str(),
            Segment::Any => !name.as_encoded_bytes().starts_with(b"."),
        }
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// A workspace's members, ordered so that each comes after the member
/// packages it depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
    members: Vec<Member>,
}

/// A project in a workspace: an application or a package with its own
/// `elm.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// Its directory, relative to the root; empty for the root itself.
    pub dir: PathBuf,
    pub outline: Outline,
    /// Its `nomi.toml`, if it has one.
    pub manifest: Option<Manifest>,
    /// The places in [`Workspace::members`] of the member packages its
    /// modules depend on.
    deps: Box<[usize]>,
    /// The places of the member packages only its tests depend on.
    test_deps: Box<[usize]>,
}

/// Why members don't make a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    /// A member whose own `nomi.toml` has a `[workspace]`.
    Nested(PathBuf),
    /// A member package by an author whose packages may hold kernel code,
    /// which only the registry and nomi provide.
    Reserved { dir: PathBuf, name: Name },
    /// Two member packages of one name, in directory order.
    Duplicate {
        name: Name,
        first: PathBuf,
        second: PathBuf,
    },
    /// Member packages that depend on each other, each on the next and the
    /// last on the first.
    Cycle(Vec<Name>),
}

impl Member {
    /// The project in `dir`, relative to the root, as its files say.
    pub fn new(dir: PathBuf, outline: Outline, manifest: Option<Manifest>) -> Member {
        Member {
            dir,
            outline,
            manifest,
            deps: Box::default(),
            test_deps: Box::default(),
        }
    }

    /// The package name, for a package.
    fn name(&self) -> Option<&Name> {
        match &self.outline {
            Outline::Pkg(pkg) => Some(&pkg.name),
            Outline::App(_) => None,
        }
    }
}

impl Workspace {
    /// The workspace at `root` of `members`, each in its own directory,
    /// with member packages named by their `elm.json` and found by the
    /// names the others depend on. Members come in dependency order, ties
    /// in the order of their directories.
    pub fn new(root: PathBuf, mut members: Vec<Member>) -> Result<Workspace, WorkspaceError> {
        members.sort_by(|a, b| a.dir.cmp(&b.dir));
        for member in &members {
            let nested = member
                .manifest
                .as_ref()
                .is_some_and(|m| m.workspace.is_some());
            if nested && !member.dir.as_os_str().is_empty() {
                return Err(WorkspaceError::Nested(member.dir.clone()));
            }
            if let Some(name) = member.name().filter(|name| name.is_kernel()) {
                return Err(WorkspaceError::Reserved {
                    dir: member.dir.clone(),
                    name: name.clone(),
                });
            }
        }
        let mut packages: Vec<(&Name, usize)> = (members.iter().enumerate())
            .filter_map(|(i, member)| Some((member.name()?, i)))
            .collect();
        packages.sort_by_key(|&(name, _)| name);
        // Sorted stably, so each name's members stay in directory order.
        if let Some(pair) = packages.windows(2).find(|pair| pair[0].0 == pair[1].0) {
            return Err(WorkspaceError::Duplicate {
                name: pair[0].0.clone(),
                first: members[pair[0].1].dir.clone(),
                second: members[pair[1].1].dir.clone(),
            });
        }
        let edges: Vec<(BTreeSet<usize>, BTreeSet<usize>)> = (members.iter())
            .map(|member| {
                let (deps, mut test) = match &member.outline {
                    Outline::App(app) => (
                        places(&packages, app.direct.keys().chain(app.indirect.keys())),
                        places(
                            &packages,
                            app.test_direct.keys().chain(app.test_indirect.keys()),
                        ),
                    ),
                    Outline::Pkg(pkg) => (
                        places(&packages, pkg.deps.keys()),
                        places(&packages, pkg.test_deps.keys()),
                    ),
                };
                test.retain(|i| !deps.contains(i));
                (deps, test)
            })
            .collect();
        let deps: Vec<Vec<usize>> = edges
            .iter()
            .map(|(deps, _)| deps.iter().copied().collect())
            .collect();
        let order = order(&deps).map_err(|cycle| {
            let name = |i: usize| {
                members[i]
                    .name()
                    .cloned()
                    .expect("a package, since another depends on it")
            };
            WorkspaceError::Cycle(cycle.into_iter().map(name).collect())
        })?;
        let mut place = vec![0; members.len()];
        for (new, &old) in order.iter().enumerate() {
            place[old] = new;
        }
        let renumber = |places: &BTreeSet<usize>| places.iter().map(|&i| place[i]).collect();
        let mut members: Vec<(usize, Member)> = (members.into_iter().zip(&edges))
            .enumerate()
            .map(|(old, (member, (deps, test)))| {
                let member = Member {
                    deps: renumber(deps),
                    test_deps: renumber(test),
                    ..member
                };
                (place[old], member)
            })
            .collect();
        members.sort_by_key(|&(new, _)| new);
        let members = members.into_iter().map(|(_, member)| member).collect();
        Ok(Workspace { root, members })
    }

    /// The directory of the root `nomi.toml`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The members, each after the member packages it depends on.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The member package named `name`.
    pub fn package(&self, name: &Name) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| member.name() == Some(name))
    }

    /// The member packages `member`'s modules depend on, directly.
    pub fn deps<'w>(&'w self, member: &'w Member) -> impl Iterator<Item = &'w Member> {
        member.deps.iter().map(|&i| &self.members[i])
    }

    /// The member packages only `member`'s tests depend on, directly.
    pub fn test_deps<'w>(&'w self, member: &'w Member) -> impl Iterator<Item = &'w Member> {
        member.test_deps.iter().map(|&i| &self.members[i])
    }

    /// The members whose modules depend on `member`, directly or through
    /// others, in the members' order.
    pub fn dependents(&self, member: &Member) -> Vec<&Member> {
        let mut reached = vec![false; self.members.len()];
        for (i, other) in self.members.iter().enumerate() {
            reached[i] =
                (other.deps.iter()).any(|&d| reached[d] || self.members[d].dir == member.dir);
        }
        (self.members.iter().zip(reached))
            .filter_map(|(other, reached)| reached.then_some(other))
            .collect()
    }
}

/// The members' places of those `names` that name member packages, where
/// `packages` pairs each member package's name, in order, with its place.
fn places<'n>(
    packages: &[(&Name, usize)],
    names: impl Iterator<Item = &'n Name>,
) -> BTreeSet<usize> {
    (names.filter_map(|name| packages.binary_search_by_key(&name, |&(n, _)| n).ok()))
        .map(|i| packages[i].1)
        .collect()
}

/// The places `0..deps.len()` ordered so that each comes after the places
/// `deps` gives it, ties smallest first, by Kahn's algorithm; or, where
/// there is no such order, a cycle: places each depending on the next and
/// the last on the first, from its smallest place.
fn order(deps: &[Vec<usize>]) -> Result<Vec<usize>, Vec<usize>> {
    let mut waiting: Vec<usize> = deps.iter().map(|d| d.len()).collect();
    let mut dependents = vec![Vec::new(); deps.len()];
    for (i, ds) in deps.iter().enumerate() {
        for &d in ds {
            dependents[d].push(i);
        }
    }
    let mut ready: BTreeSet<usize> = (0..deps.len()).filter(|&i| waiting[i] == 0).collect();
    let mut order = Vec::with_capacity(deps.len());
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for &dependent in &dependents[i] {
            waiting[dependent] -= 1;
            if waiting[dependent] == 0 {
                ready.insert(dependent);
            }
        }
    }
    if order.len() == deps.len() {
        return Ok(order);
    }
    // Every place left waits on another left, so following the first such
    // dependency from any of them comes back round.
    let left = |i: usize| waiting[i] > 0;
    let mut path = vec![waiting.iter().position(|&w| w > 0).expect("a place left")];
    loop {
        let last = *path.last().expect("a place");
        let next = *deps[last]
            .iter()
            .find(|&&d| left(d))
            .expect("a dependency left");
        if let Some(start) = path.iter().position(|&i| i == next) {
            let mut cycle = path.split_off(start);
            let smallest = (0..cycle.len()).min_by_key(|&i| cycle[i]).expect("a place");
            cycle.rotate_left(smallest);
            return Err(cycle);
        }
        path.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_patterns_are_refused() {
        for text in [
            "",
            ".",
            "./",
            "/abs",
            "../x",
            "apps/../x",
            "a*b",
            "apps/web*",
            "**",
        ] {
            assert_eq!(Pattern::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_pattern_matches_whole_components() {
        let cases: &[(&str, &str, bool)] = &[
            ("apps/web", "apps/web", true),
            ("apps/web", "./apps/web", true),
            ("./apps/web/", "apps/web", true),
            ("apps/web", "apps/api", false),
            ("apps/web", "apps", false),
            ("apps/web", "apps/web/src", false),
            ("packages/*", "packages/shared", true),
            ("packages/*", "packages/.hidden", false),
            ("packages/*", "packages", false),
            ("packages/*", "packages/a/b", false),
            ("*/app", "web/app", true),
            ("*/app", "web/lib", false),
            ("apps/web", "../apps/web", false),
        ];
        for &(pattern, path, expected) in cases {
            let found = Pattern::parse(pattern)
                .expect(pattern)
                .matches(Path::new(path));
            assert_eq!(found, expected, "{pattern} against {path}");
        }
    }

    #[test]
    fn a_pattern_expands_through_the_directories_listed() {
        let subdirs = |dir: &Path| -> io::Result<Vec<OsString>> {
            let names: &[&str] = match dir.to_str() {
                Some("") => &["apps", "packages"],
                Some("apps") => &["api", "web"],
                Some("packages") => &[".hidden", "shared"],
                _ => &[],
            };
            Ok(names.iter().map(OsString::from).collect())
        };
        let cases: &[(&str, &[&str])] = &[
            ("apps/web", &["apps/web"]),
            ("missing/dir", &["missing/dir"]),
            ("apps/*", &["apps/api", "apps/web"]),
            ("packages/*", &["packages/shared"]),
            ("*/web", &["apps/web", "packages/web"]),
            ("*/*", &["apps/api", "apps/web", "packages/shared"]),
        ];
        for &(pattern, expected) in cases {
            let pattern = Pattern::parse(pattern).expect("a good pattern");
            let found = pattern.expand(subdirs).expect("listed");
            let expected: Vec<PathBuf> = expected.iter().map(PathBuf::from).collect();
            assert_eq!(found, expected, "{pattern}");
            assert_eq!(pattern.is_literal(), !pattern.text.contains('*'));
        }
    }

    fn member(dir: &str, elm_json: &str) -> Member {
        let (outline, _) = Outline::detect(elm_json.as_bytes()).expect("a good elm.json");
        Member::new(PathBuf::from(dir), outline, None)
    }

    #[test]
    fn members_know_their_dependencies_and_dependents() {
        use nomi_testkit::workspace::{app, package};
        let workspace = Workspace::new(
            PathBuf::from("/ws"),
            vec![
                member("apps/web", &app(&["acme/ui"], &["acme/check"])),
                member("apps/api", &app(&["acme/core", "elm/http"], &[])),
                member("packages/ui", &package("acme/ui", &["acme/core"], &[])),
                member("packages/core", &package("acme/core", &[], &["acme/check"])),
                member("packages/check", &package("acme/check", &[], &[])),
            ],
        )
        .expect("a workspace");
        let dirs = |members: Vec<&Member>| -> Vec<String> {
            (members.iter())
                .map(|m| m.dir.to_string_lossy().into_owned())
                .collect()
        };
        let all: Vec<&Member> = workspace.members().iter().collect();
        assert_eq!(
            dirs(all),
            [
                "packages/check",
                "packages/core",
                "apps/api",
                "packages/ui",
                "apps/web"
            ]
        );
        let named = |name: &str| {
            let name: Name = name.parse().expect("a name");
            workspace.package(&name).expect("a member package")
        };
        let (check, core, ui) = (named("acme/check"), named("acme/core"), named("acme/ui"));
        let web = &workspace.members()[4];
        assert_eq!(dirs(workspace.deps(web).collect()), ["packages/ui"]);
        assert_eq!(dirs(workspace.test_deps(web).collect()), ["packages/check"]);
        assert_eq!(dirs(workspace.deps(ui).collect()), ["packages/core"]);
        assert_eq!(
            dirs(workspace.dependents(core)),
            ["apps/api", "packages/ui", "apps/web"]
        );
        assert_eq!(dirs(workspace.dependents(check)), Vec::<String>::new());
        assert_eq!(dirs(workspace.dependents(web)), Vec::<String>::new());
        assert!(workspace.package(&Name::core()).is_none());
    }

    #[test]
    fn a_pattern_shows_as_written() {
        let pattern = Pattern::parse("./packages/*/").expect("a good pattern");
        assert_eq!(pattern.to_string(), "./packages/*/");
    }
}
