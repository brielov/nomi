//! `Solver.initEnv`: the package index, brought up to date when the
//! website answers, and the releases the solver reads, for the commands
//! that change a project's dependencies.

use nomi_base::{Compat, Report};
use nomi_registry::{Http, Index, LatestError, Registry, Store, report};

use crate::Context;
use crate::console::{self, Console};
use crate::packages::{self, Resolve};
use crate::paths::Cache;
use crate::project::Project;

/// Runs `body` on the project around the directory, loaded for `purpose`,
/// and the registry, which `context` needed, reporting a problem in the
/// project's release. Whether it succeeded.
pub(crate) fn with_registry<H: Http>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    purpose: &str,
    context: &str,
    body: impl FnOnce(&mut dyn Console, &Project, &mut Registry<'_, H>) -> Result<bool, Report>,
) -> bool {
    let mut compat = Compat::LATEST;
    let run = |console: &mut dyn Console| {
        let project = Project::load(cx.vfs, cx.dir, purpose)?;
        compat = project.compat;
        let store = packages::store_for(cx.cache, Resolve::Nomi)?;
        let mut registry = registry(cx.cache, &store, cx.http, compat, context)?;
        body(console, &project, &mut registry)
    };
    let result = run(console);
    console::reported(console, result, compat)
}

/// The registry the solver searches, or the report of why there is none,
/// which `context` needed.
pub(crate) fn registry<'a, H: Http>(
    cache: &Cache,
    store: &'a Store,
    http: &'a H,
    compat: Compat,
    context: &str,
) -> Result<Registry<'a, H>, Report> {
    match Index::latest(http, &cache.index(), compat) {
        Ok((index, connection)) => Ok(Registry::new(
            index,
            connection,
            store,
            cache.metadata(),
            http,
            compat,
        )),
        Err(e) => Err(latest_problem("PROBLEM LOADING PACKAGE LIST", context, e)),
    }
}

/// Why there is no package index, which `context` needed, under `title`.
pub(crate) fn latest_problem(title: &'static str, context: &str, error: LatestError) -> Report {
    match error {
        LatestError::Registry(problem) => report::registry_problem(title, &problem, context),
        LatestError::Io(e) => Report::new(
            title,
            None,
            &format!("{context}, but I could not read or write the copy kept in the cache: {e}"),
            [],
        ),
    }
}
