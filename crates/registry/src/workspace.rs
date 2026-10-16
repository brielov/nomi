//! One resolution for a workspace: every application's pins and every
//! member package's ranges solved together, so that the members build
//! with one release of each package. A member package is the one release
//! of its name, taken from the workspace, as the bundle's are: the
//! registry's releases of that name, if it has any, are never asked about.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::PathBuf;

use nomi_base::Compat;
use nomi_project::outline::AppOutline;
use nomi_project::workspace::{Member, Workspace};
use nomi_project::{Constraint, Name, Outline, Version};

use crate::details::{self, DetailsProblem};
use crate::index::Known;
use crate::solver::{Details, Packages, Release};

/// Why a workspace's members don't resolve together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceProblem<E> {
    /// Two applications are for different releases of elm, where one
    /// resolution needs one compiler.
    MixedElm { first: Pin, second: Pin },
    /// Two applications pin different releases of `name`.
    Disagree { name: Name, first: Pin, second: Pin },
    /// An application, `by`, pins the member package `name` at a release
    /// other than `member`, the workspace's.
    MemberVersion { name: Name, member: Pin, by: Pin },
    /// A package, the member in `by`, depends on the member package
    /// `name` in a range that leaves out `member`, the workspace's
    /// release.
    MemberRange {
        name: Name,
        member: Pin,
        by: PathBuf,
        range: Constraint,
    },
    /// The members' dependencies don't check out together, or the
    /// project's own don't within them.
    Details(DetailsProblem<E>),
}

/// A member, by its directory relative to the root, and the release it
/// names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub dir: PathBuf,
    pub version: Version,
}

/// The releases the member whose `elm.json` is `outline` builds with:
/// its part of the one resolution of `workspace`, which `packages` and
/// the member packages give, checked as upstream checks a project's
/// dependencies.
pub fn verify<P: Packages>(
    packages: &mut P,
    compat: Compat,
    workspace: &Workspace,
    outline: &Outline,
) -> Result<BTreeMap<Name, Details>, WorkspaceProblem<P::Error>> {
    agree(workspace)?;
    let mut members = Members::new(workspace, packages);
    let solution = match constraints(workspace) {
        Some(constraints) => details::solve(&mut members, compat, &constraints),
        None => Err(details::no_solution(&members)),
    };
    let held = (solution.map_err(WorkspaceProblem::Details)?.into_iter())
        .map(|(name, details)| {
            let known = Known {
                newest: details.version,
                previous: Vec::new(),
            };
            (name, known)
        })
        .collect();
    let mut within = Within {
        held,
        packages: &mut members,
    };
    details::verify(&mut within, compat, outline).map_err(WorkspaceProblem::Details)
}

/// That the applications are for one elm, that every member asks for
/// each member package at the workspace's release of it, and that the
/// applications pin one release of each package.
pub fn agree<E>(workspace: &Workspace) -> Result<(), WorkspaceProblem<E>> {
    let pin = |member: &Member, version| Pin {
        dir: member.dir.clone(),
        version,
    };
    let apps: Vec<(&Member, &AppOutline)> = (workspace.members().iter())
        .filter_map(|member| match &member.outline {
            Outline::App(app) => Some((member, app)),
            Outline::Pkg(_) => None,
        })
        .collect();
    if let Some(&(first, app)) = apps.first()
        && let Some(&(second, other)) = apps.iter().find(|(_, other)| other.elm != app.elm)
    {
        return Err(WorkspaceProblem::MixedElm {
            first: pin(first, app.elm),
            second: pin(second, other.elm),
        });
    }
    let releases: BTreeMap<&Name, Pin> = (workspace.members().iter())
        .filter_map(|member| match &member.outline {
            Outline::Pkg(pkg) => Some((&pkg.name, pin(member, pkg.version))),
            Outline::App(_) => None,
        })
        .collect();
    for member in workspace.members() {
        match &member.outline {
            Outline::App(app) => {
                for (name, version) in app.all_deps() {
                    if let Some(release) = releases.get(&name)
                        && release.version != version
                    {
                        return Err(WorkspaceProblem::MemberVersion {
                            name,
                            member: release.clone(),
                            by: pin(member, version),
                        });
                    }
                }
            }
            Outline::Pkg(pkg) => {
                for (name, range) in pkg.deps.iter().chain(&pkg.test_deps) {
                    if let Some(release) = releases.get(name)
                        && !range.satisfies(release.version)
                    {
                        return Err(WorkspaceProblem::MemberRange {
                            name: name.clone(),
                            member: release.clone(),
                            by: member.dir.clone(),
                            range: *range,
                        });
                    }
                }
            }
        }
    }
    let mut pinned: BTreeMap<Name, Pin> = BTreeMap::new();
    for (member, app) in apps {
        for (name, version) in app.all_deps() {
            match pinned.entry(name) {
                Entry::Vacant(entry) => {
                    entry.insert(pin(member, version));
                }
                Entry::Occupied(entry) if entry.get().version != version => {
                    return Err(WorkspaceProblem::Disagree {
                        name: entry.key().clone(),
                        first: entry.get().clone(),
                        second: pin(member, version),
                    });
                }
                Entry::Occupied(_) => {}
            }
        }
    }
    Ok(())
}

/// What the members need together: each application's pins exactly, and
/// each member package's ranges, its tests' among them; `None` if two
/// rule each other out.
fn constraints(workspace: &Workspace) -> Option<BTreeMap<Name, Constraint>> {
    let mut constraints: BTreeMap<Name, Constraint> = BTreeMap::new();
    for (name, constraint) in workspace.members().iter().flat_map(asks) {
        match constraints.entry(name) {
            Entry::Vacant(entry) => {
                entry.insert(constraint);
            }
            Entry::Occupied(mut entry) => {
                let both = entry.get().intersect(&constraint)?;
                entry.insert(both);
            }
        }
    }
    Some(constraints)
}

/// What `member` asks for: an application's pins, exactly, or a
/// package's ranges, its tests' among them.
fn asks(member: &Member) -> Vec<(Name, Constraint)> {
    match &member.outline {
        Outline::App(app) => (app.all_deps().into_iter())
            .map(|(name, version)| (name, Constraint::exactly(version)))
            .collect(),
        Outline::Pkg(pkg) => (pkg.deps.iter().chain(&pkg.test_deps))
            .map(|(name, &constraint)| (name.clone(), constraint))
            .collect(),
    }
}

/// The releases `packages` has, with each member package of a workspace
/// the one release of its name, and each package another application
/// pins, where some are held, only at that pin.
#[derive(Debug)]
pub struct Members<'p, P> {
    members: BTreeMap<Name, (Known, Release)>,
    held: BTreeMap<Name, Known>,
    packages: &'p mut P,
}

impl<'p, P: Packages> Members<'p, P> {
    /// The releases of `packages`, with the member packages of
    /// `workspace`.
    pub fn new(workspace: &Workspace, packages: &'p mut P) -> Members<'p, P> {
        let members = (workspace.members().iter())
            .filter_map(|member| match &member.outline {
                Outline::Pkg(pkg) => {
                    let known = Known {
                        newest: pkg.version,
                        previous: Vec::new(),
                    };
                    let release = Release {
                        elm: pkg.elm,
                        deps: pkg.deps.clone(),
                    };
                    Some((pkg.name.clone(), (known, release)))
                }
                Outline::App(_) => None,
            })
            .collect();
        Members {
            members,
            held: BTreeMap::new(),
            packages,
        }
    }

    /// The releases with each package an application of `workspace` pins,
    /// other than those in `dirs`, held at its pin, so that what those
    /// move to keeps agreeing with the rest.
    pub fn holding_all_but(mut self, workspace: &Workspace, dirs: &[PathBuf]) -> Members<'p, P> {
        for member in workspace.members() {
            if let Outline::App(app) = &member.outline
                && !dirs.contains(&member.dir)
            {
                let held = (app.all_deps().into_iter()).map(|(name, version)| {
                    let known = Known {
                        newest: version,
                        previous: Vec::new(),
                    };
                    (name, known)
                });
                self.held.extend(held);
            }
        }
        self
    }
}

impl<P: Packages> Packages for Members<'_, P> {
    type Error = P::Error;

    fn versions(&self, name: &Name) -> Option<&Known> {
        match self.members.get(name) {
            Some((known, _)) => Some(known),
            None => self.held.get(name).or_else(|| self.packages.versions(name)),
        }
    }

    fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, P::Error> {
        match self.members.get(name) {
            Some((known, release)) => Ok((known.newest == version).then(|| release.clone())),
            None => self.packages.release(name, version),
        }
    }

    fn offline(&self) -> bool {
        self.packages.offline()
    }
}

/// The releases of a solution, each the one release of its name, and
/// nothing else.
struct Within<'p, P> {
    held: BTreeMap<Name, Known>,
    packages: &'p mut P,
}

impl<P: Packages> Packages for Within<'_, P> {
    type Error = P::Error;

    fn versions(&self, name: &Name) -> Option<&Known> {
        self.held.get(name)
    }

    fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, P::Error> {
        self.packages.release(name, version)
    }

    fn offline(&self) -> bool {
        self.packages.offline()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;

    use nomi_testkit::workspace::{app, package};
    use serde_json::Value;

    use super::*;
    use crate::fixture::{Fixture, name, version as v};
    use crate::install::{self, Change, Plan};
    use crate::update;

    const COMPAT: Compat = Compat::V0_19_1;

    /// The fixture's releases, remembering each package asked about.
    struct Asked {
        fixture: Fixture,
        asked: RefCell<Vec<Name>>,
    }

    impl Packages for Asked {
        type Error = ();

        fn versions(&self, name: &Name) -> Option<&Known> {
            self.asked.borrow_mut().push(name.clone());
            self.fixture.versions(name)
        }

        fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, ()> {
            self.asked.borrow_mut().push(name.clone());
            self.fixture.release(name, version)
        }
    }

    /// elm/core 1.0.5 and newer, elm/json 1.1.3 and older, elm/html,
    /// elm/url 1.0.0 and newer, and a published acme/shared, newer than
    /// the member and needing elm/html.
    fn index() -> Asked {
        let known = |all: &[&str]| Known {
            newest: v(all[0]),
            previous: all[1..].iter().map(|s| v(s)).collect(),
        };
        let needs = |deps: &[&str]| deps.iter().map(|d| name(d)).collect();
        let fixture = Fixture {
            index: BTreeMap::from([
                (name("elm/core"), known(&["1.0.6", "1.0.5"])),
                (name("elm/json"), known(&["1.1.3", "1.1.2"])),
                (name("elm/html"), known(&["1.0.0"])),
                (name("elm/url"), known(&["1.0.1", "1.0.0"])),
                (name("acme/shared"), known(&["2.0.0", "1.0.0"])),
            ]),
            needs: BTreeMap::from([
                (name("elm/core"), needs(&[])),
                (name("elm/json"), needs(&["elm/core"])),
                (name("elm/html"), needs(&["elm/core"])),
                (name("elm/url"), needs(&["elm/core"])),
                (name("acme/shared"), needs(&["elm/html"])),
            ]),
        };
        Asked {
            fixture,
            asked: RefCell::new(Vec::new()),
        }
    }

    /// `elm_json` with the field at `path` set to `value`.
    fn edited(elm_json: &str, path: &[&str], value: &str) -> String {
        let mut json: Value = serde_json::from_str(elm_json).expect("JSON");
        let field = (path.iter()).fold(&mut json, |json, &key| &mut json[key]);
        *field = Value::from(value);
        json.to_string()
    }

    fn pin(dir: &str, version: &str) -> Pin {
        Pin {
            dir: PathBuf::from(dir),
            version: v(version),
        }
    }

    /// A workspace, by each member's directory and `elm.json`; the member
    /// whose releases are asked for; and the answer.
    struct Case {
        name: &'static str,
        members: Vec<(&'static str, String)>,
        project: &'static str,
        expected: Result<Vec<(Name, Version)>, WorkspaceProblem<()>>,
    }

    impl Case {
        /// Checks the answer, and that the index was never asked about the
        /// member package.
        fn check(self) {
            let members = (self.members.iter())
                .map(|(dir, elm_json)| {
                    let (outline, _) = Outline::detect(elm_json.as_bytes()).expect("an elm.json");
                    Member::new(PathBuf::from(dir), outline, None)
                })
                .collect();
            let workspace = Workspace::new(PathBuf::from("/ws"), members).expect("a workspace");
            let member = (workspace.members().iter())
                .find(|m| m.dir == Path::new(self.project))
                .expect("a member");
            let mut index = index();
            let answer = verify(&mut index, COMPAT, &workspace, &member.outline).map(|solution| {
                (solution.into_iter())
                    .map(|(name, details)| (name, details.version))
                    .collect()
            });
            assert_eq!(answer, self.expected, "{}", self.name);
            let asked = index.asked.into_inner();
            assert!(
                !asked.contains(&name("acme/shared")),
                "{}: {asked:?}",
                self.name
            );
        }
    }

    fn web() -> String {
        app(&["acme/shared"], &[])
    }

    fn api() -> String {
        app(&[], &[])
    }

    fn shared() -> String {
        package("acme/shared", &["elm/json"], &[])
    }

    #[test]
    fn members_that_agree_resolve_together() {
        let releases =
            |all: &[(&str, &str)]| Ok(all.iter().map(|&(n, at)| (name(n), v(at))).collect());
        let cases = [
            Case {
                name: "a package within the apps' pins",
                members: vec![
                    ("apps/web", web()),
                    ("apps/api", api()),
                    ("packages/shared", shared()),
                ],
                project: "packages/shared",
                expected: releases(&[("elm/core", "1.0.5"), ("elm/json", "1.1.3")]),
            },
            Case {
                name: "an app using a member shadowing a newer published release",
                members: vec![("apps/web", web()), ("packages/shared", shared())],
                project: "apps/web",
                expected: releases(&[
                    ("acme/shared", "1.0.0"),
                    ("elm/core", "1.0.5"),
                    ("elm/json", "1.1.3"),
                ]),
            },
        ];
        cases.into_iter().for_each(Case::check);
    }

    #[test]
    fn members_that_dont_agree_say_why() {
        let cases = [
            Case {
                name: "apps that disagree",
                members: vec![
                    ("apps/web", web()),
                    (
                        "apps/api",
                        edited(&api(), &["dependencies", "direct", "elm/json"], "1.1.2"),
                    ),
                    ("packages/shared", shared()),
                ],
                project: "apps/web",
                expected: Err(WorkspaceProblem::Disagree {
                    name: name("elm/json"),
                    first: pin("apps/api", "1.1.2"),
                    second: pin("apps/web", "1.1.3"),
                }),
            },
            Case {
                name: "an app pinning another release of a member",
                members: vec![
                    (
                        "apps/web",
                        edited(&web(), &["dependencies", "direct", "acme/shared"], "2.0.0"),
                    ),
                    ("packages/shared", shared()),
                ],
                project: "apps/web",
                expected: Err(WorkspaceProblem::MemberVersion {
                    name: name("acme/shared"),
                    member: pin("packages/shared", "1.0.0"),
                    by: pin("apps/web", "2.0.0"),
                }),
            },
            Case {
                name: "a package whose range leaves out a member's release",
                members: vec![
                    (
                        "packages/ui",
                        edited(
                            &package("acme/ui", &["acme/shared"], &[]),
                            &["dependencies", "acme/shared"],
                            "2.0.0 <= v < 3.0.0",
                        ),
                    ),
                    ("packages/shared", shared()),
                ],
                project: "packages/ui",
                expected: Err(WorkspaceProblem::MemberRange {
                    name: name("acme/shared"),
                    member: pin("packages/shared", "1.0.0"),
                    by: PathBuf::from("packages/ui"),
                    range: Constraint::until_next_major(v("2.0.0"), COMPAT),
                }),
            },
            Case {
                name: "apps for different releases of elm",
                members: vec![
                    ("apps/web", web()),
                    ("apps/api", edited(&api(), &["elm-version"], "0.19.2")),
                ],
                project: "apps/web",
                expected: Err(WorkspaceProblem::MixedElm {
                    first: pin("apps/api", "0.19.2"),
                    second: pin("apps/web", "0.19.1"),
                }),
            },
            Case {
                name: "a package's range leaving out an app's pin",
                members: vec![
                    ("apps/web", web()),
                    (
                        "packages/shared",
                        edited(
                            &shared(),
                            &["dependencies", "elm/json"],
                            "1.1.4 <= v < 2.0.0",
                        ),
                    ),
                ],
                project: "packages/shared",
                expected: Err(WorkspaceProblem::Details(DetailsProblem::NoSolution)),
            },
        ];
        cases.into_iter().for_each(Case::check);
    }

    /// A workspace at `/ws` of `members`, each by its directory and
    /// `elm.json`.
    fn workspace(members: &[(&str, String)]) -> Workspace {
        let members = (members.iter())
            .map(|(dir, elm_json)| {
                let (outline, _) = Outline::detect(elm_json.as_bytes()).expect("an elm.json");
                Member::new(PathBuf::from(dir), outline, None)
            })
            .collect();
        Workspace::new(PathBuf::from("/ws"), members).expect("a workspace")
    }

    fn app_of<'w>(workspace: &'w Workspace, dir: &str) -> &'w AppOutline {
        let member = (workspace.members().iter()).find(|m| m.dir == Path::new(dir));
        match &member.expect("a member").outline {
            Outline::App(app) => app,
            Outline::Pkg(_) => panic!("{dir} is a package"),
        }
    }

    #[test]
    fn an_application_adding_a_package_takes_what_the_others_pin() {
        let workspace = workspace(&[("apps/api", api()), ("apps/web", app(&["elm/url"], &[]))]);
        let outline = Outline::App(app_of(&workspace, "apps/api").clone());
        let added = |held: bool| {
            let mut index = index();
            let mut members = Members::new(&workspace, &mut index);
            if held {
                members = members.holding_all_but(&workspace, &[PathBuf::from("apps/api")]);
            }
            match install::plan(&mut members, COMPAT, &name("elm/url"), &outline) {
                Ok(Plan::App { outline, .. }) => outline.direct[&name("elm/url")],
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(added(false), v("1.0.1"));
        assert_eq!(added(true), v("1.0.0"));
    }

    #[test]
    fn applications_updated_together_move_together_and_hold_the_rest() {
        let workspace = workspace(&[
            ("apps/api", api()),
            ("apps/web", web()),
            ("packages/shared", shared()),
        ]);
        let apps = |dirs: &[&str]| -> Vec<(PathBuf, &AppOutline)> {
            (dirs.iter())
                .map(|&dir| (PathBuf::from(dir), app_of(&workspace, dir)))
                .collect()
        };
        let both = ["apps/api", "apps/web"].map(PathBuf::from);
        let mut releases = index();
        let mut members =
            Members::new(&workspace, &mut releases).holding_all_but(&workspace, &both);
        let together =
            update::plan_together(&mut members, COMPAT, None, &apps(&["apps/api", "apps/web"]))
                .expect("a plan");
        assert_eq!(
            together.changes,
            BTreeMap::from([(
                name("elm/core"),
                Change::Change {
                    old: v("1.0.5"),
                    new: v("1.0.6")
                }
            )])
        );
        let cores: Vec<Version> = (together.apps.iter())
            .map(|(_, app)| app.direct[&name("elm/core")])
            .collect();
        assert_eq!(cores, [v("1.0.6"), v("1.0.6")]);
        // web keeps acme/shared, the member, which api never had.
        let (_, web) = &together.apps[1];
        assert_eq!(web.direct.get(&name("acme/shared")), Some(&v("1.0.0")));
        assert!(!together.apps[0].1.direct.contains_key(&name("acme/shared")));
        // web alone holds what api pins.
        let mut releases = index();
        let web_only = [PathBuf::from("apps/web")];
        let mut members =
            Members::new(&workspace, &mut releases).holding_all_but(&workspace, &web_only);
        let alone = update::plan_together(&mut members, COMPAT, None, &apps(&["apps/web"]))
            .expect("a plan");
        assert!(alone.changes.is_empty(), "{:?}", alone.changes);
    }
}
