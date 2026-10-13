//! Why there is no solution, which upstream doesn't say. `PubGrub` searches
//! the same releases and, when it finds none, derives the reason.

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::ops::Bound;

use nomi_base::Compat;
use nomi_project::{Constraint, Name, Op, Version};
use pubgrub::{
    Dependencies, DependencyConstraints, DependencyProvider, Derived, External, Map,
    PackageResolutionStatistics, PubGrubError, Ranges, ReportFormatter, Reporter, Term,
};

use super::{Packages, Releases};

/// Why no releases satisfy `constraints` and everything they need, or
/// `None` if some do, or if reading a release failed along the way.
pub fn explain<P: Packages>(
    packages: &mut P,
    compat: Compat,
    constraints: &BTreeMap<Name, Constraint>,
) -> Option<String> {
    let provider = Provider {
        releases: RefCell::new(Releases::new(packages)),
        compat,
        constraints,
    };
    match pubgrub::resolve(&provider, Pkg::Project, Version::compiler(compat)) {
        Err(PubGrubError::NoSolution(tree)) => Some(
            pubgrub::DefaultStringReporter::report_with_formatter(&tree, &Formatter),
        ),
        _ => None,
    }
}

/// A package to `PubGrub`: the project's dependencies are a package's.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Pkg {
    Project,
    Package(Name),
}

impl fmt::Display for Pkg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pkg::Project => f.write_str("the project"),
            Pkg::Package(name) => name.fmt(f),
        }
    }
}

/// Why a release can't be used.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Unusable {
    /// [`Packages::release`] has no metadata for it.
    Missing,
    /// Its `elm-version` excludes the compiler.
    Elm(Constraint),
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unusable::Missing => f.write_str("is not in the package cache"),
            Unusable::Elm(elm) => write!(f, "needs elm {elm}"),
        }
    }
}

/// Reading a release failed, which ends the explanation.
#[derive(Debug)]
struct Unreadable;

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a release's metadata is unreadable")
    }
}

impl Error for Unreadable {}

struct Provider<'a, P: Packages> {
    releases: RefCell<Releases<'a, P>>,
    compat: Compat,
    constraints: &'a BTreeMap<Name, Constraint>,
}

impl<P: Packages> DependencyProvider for Provider<'_, P> {
    type P = Pkg;
    type V = Version;
    type VS = Ranges<Version>;
    type M = Unusable;
    /// Upstream's order: the first name first.
    type Priority = Reverse<Pkg>;
    type Err = Unreadable;

    fn prioritize(
        &self,
        package: &Pkg,
        _: &Ranges<Version>,
        _: &PackageResolutionStatistics,
    ) -> Reverse<Pkg> {
        Reverse(package.clone())
    }

    /// The newest release in `range`.
    fn choose_version(
        &self,
        package: &Pkg,
        range: &Ranges<Version>,
    ) -> Result<Option<Version>, Unreadable> {
        Ok(match package {
            Pkg::Project => Some(Version::compiler(self.compat)),
            Pkg::Package(name) => self
                .releases
                .borrow()
                .packages
                .versions(name)
                .and_then(|known| known.all().find(|v| range.contains(v))),
        })
    }

    fn get_dependencies(
        &self,
        package: &Pkg,
        version: &Version,
    ) -> Result<Dependencies<Pkg, Ranges<Version>, Unusable>, Unreadable> {
        let name = match package {
            Pkg::Project => return Ok(Dependencies::Available(dependencies(self.constraints))),
            Pkg::Package(name) => name,
        };
        let mut releases = self.releases.borrow_mut();
        let release = releases.get(name, *version).map_err(|_| Unreadable)?;
        Ok(match release {
            None => Dependencies::Unavailable(Unusable::Missing),
            Some(release) if !release.elm.satisfies(Version::compiler(self.compat)) => {
                Dependencies::Unavailable(Unusable::Elm(release.elm))
            }
            Some(release) => Dependencies::Available(dependencies(&release.deps)),
        })
    }
}

fn dependencies(
    constraints: &BTreeMap<Name, Constraint>,
) -> DependencyConstraints<Pkg, Ranges<Version>> {
    constraints
        .iter()
        .map(|(name, &constraint)| (Pkg::Package(name.clone()), range(constraint)))
        .collect()
}

fn range(constraint: Constraint) -> Ranges<Version> {
    let bound = |op, version| match op {
        Op::Less => Bound::Excluded(version),
        Op::LessOrEqual => Bound::Included(version),
    };
    Ranges::from_range_bounds((
        bound(constraint.lower_op, constraint.lower),
        bound(constraint.upper_op, constraint.upper),
    ))
}

/// Releases of `package` in `range`, in Elm's words where they fit.
fn releases(package: &Pkg, range: &Ranges<Version>) -> String {
    if *package == Pkg::Project {
        return package.to_string();
    }
    if let Some(version) = range.as_singleton() {
        return format!("{package} {version}");
    }
    let mut segments = range.iter();
    let interval = match (segments.next(), segments.next()) {
        (Some(segment), None) => Some(segment),
        _ => None,
    };
    let lower = |bound: &Bound<Version>| match bound {
        Bound::Included(v) => format!("{v} <= "),
        Bound::Excluded(v) => format!("{v} < "),
        Bound::Unbounded => String::new(),
    };
    let upper = |bound: &Bound<Version>| match bound {
        Bound::Included(v) => format!(" <= {v}"),
        Bound::Excluded(v) => format!(" < {v}"),
        Bound::Unbounded => String::new(),
    };
    match interval {
        Some((Bound::Unbounded, Bound::Unbounded)) => format!("every release of {package}"),
        Some((low, high)) => format!("{package} {}v{}", lower(low), upper(high)),
        None => format!("{package} {range}"),
    }
}

/// `DefaultStringReportFormatter`, but for the project and Elm's
/// constraints.
struct Formatter;

type Terms = Map<Pkg, Term<Ranges<Version>>>;
type Ext = External<Pkg, Ranges<Version>, Unusable>;
type Der = Derived<Pkg, Ranges<Version>, Unusable>;

impl ReportFormatter<Pkg, Ranges<Version>, Unusable> for Formatter {
    type Output = String;

    fn format_external(&self, external: &Ext) -> String {
        match external {
            External::NotRoot(..) => "the project is what's being solved".to_owned(),
            External::NoVersions(package, range) => {
                if *range == Ranges::full() {
                    format!("{package} has no releases")
                } else {
                    format!("there is no {}", releases(package, range))
                }
            }
            External::FromDependencyOf(package, range, dep, dep_range) => {
                format!(
                    "{} depends on {}",
                    releases(package, range),
                    releases(dep, dep_range)
                )
            }
            External::Custom(package, range, unusable) => {
                format!("{} {unusable}", releases(package, range))
            }
        }
    }

    fn format_terms(&self, terms: &Terms) -> String {
        let terms: Vec<_> = terms.iter().collect();
        match terms.as_slice() {
            [] => "no releases work".to_owned(),
            [(Pkg::Project, Term::Positive(_))] => "the project can't be solved".to_owned(),
            [(package, Term::Positive(range))] => {
                format!("{} can't be used", releases(package, range))
            }
            [(package, Term::Negative(range))] => {
                format!("{} is needed", releases(package, range))
            }
            [(p1, Term::Positive(r1)), (p2, Term::Negative(r2))]
            | [(p2, Term::Negative(r2)), (p1, Term::Positive(r1))] => self.format_external(
                &External::FromDependencyOf(Pkg::clone(p1), r1.clone(), Pkg::clone(p2), r2.clone()),
            ),
            terms => {
                let terms: Vec<_> = terms
                    .iter()
                    .map(|(package, term)| match term {
                        Term::Positive(range) => releases(package, range),
                        Term::Negative(range) => format!("not {}", releases(package, range)),
                    })
                    .collect();
                format!("{} can't be used together", terms.join(", "))
            }
        }
    }

    fn explain_both_external(&self, e1: &Ext, e2: &Ext, terms: &Terms) -> String {
        format!(
            "Because {} and {}, {}.",
            self.format_external(e1),
            self.format_external(e2),
            self.format_terms(terms)
        )
    }

    fn explain_both_ref(
        &self,
        id1: usize,
        d1: &Der,
        id2: usize,
        d2: &Der,
        terms: &Terms,
    ) -> String {
        format!(
            "Because {} ({id1}) and {} ({id2}), {}.",
            self.format_terms(&d1.terms),
            self.format_terms(&d2.terms),
            self.format_terms(terms)
        )
    }

    fn explain_ref_and_external(&self, id: usize, d: &Der, e: &Ext, terms: &Terms) -> String {
        format!(
            "Because {} ({id}) and {}, {}.",
            self.format_terms(&d.terms),
            self.format_external(e),
            self.format_terms(terms)
        )
    }

    fn and_explain_external(&self, e: &Ext, terms: &Terms) -> String {
        format!(
            "And because {}, {}.",
            self.format_external(e),
            self.format_terms(terms)
        )
    }

    fn and_explain_ref(&self, id: usize, d: &Der, terms: &Terms) -> String {
        format!(
            "And because {} ({id}), {}.",
            self.format_terms(&d.terms),
            self.format_terms(terms)
        )
    }

    fn and_explain_prior_and_external(&self, prior: &Ext, e: &Ext, terms: &Terms) -> String {
        format!(
            "And because {} and {}, {}.",
            self.format_external(prior),
            self.format_external(e),
            self.format_terms(terms)
        )
    }
}
