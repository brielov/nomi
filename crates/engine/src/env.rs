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
    const TITLE: &str = "PROBLEM LOADING PACKAGE LIST";
    match Index::latest(http, &cache.index(), compat) {
        Ok((index, connection)) => Ok(Registry::new(
            index,
            connection,
            store,
            cache.metadata(),
            http,
            compat,
        )),
        Err(LatestError::Registry(problem)) => {
            Err(report::registry_problem(TITLE, &problem, context))
        }
        Err(LatestError::Io(e)) => Err(Report::new(
            TITLE,
            None,
            &format!("{context}, but I could not read or write the copy kept in the cache: {e}"),
            [],
        )),
    }
}
