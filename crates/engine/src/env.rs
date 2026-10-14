//! `Solver.initEnv`: the package index, brought up to date when the
//! website answers, and the releases the solver reads, for the commands
//! that change a project's dependencies.

use nomi_base::{Compat, Report};
use nomi_registry::{Http, Index, LatestError, Registry, Store, report};

use crate::paths::Cache;

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
