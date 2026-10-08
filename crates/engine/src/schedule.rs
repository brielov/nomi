//! Running the module DAG on the pool (architecture §6). A module starts
//! once every module it imports has finished, on whichever thread is free,
//! so a build is as parallel as its imports allow. A module whose imports
//! failed doesn't run: upstream reports only the earliest failing phase,
//! and the modules it blocks stay silent.
//!
//! Each module's work is a phase between checks of the snapshot: once the
//! snapshot is stale, no further module starts, and the run ends
//! [`Cancelled`] when the ones running finish.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::{Pool, Snapshot};

/// What came of one module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<T, E> {
    Done(T),
    Failed(E),
    /// A module it imports failed or was blocked, so it didn't run.
    Blocked,
}

/// The snapshot went stale before every module ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

/// Runs `job` on each module of the DAG whose module `i` imports
/// `imports[i]`, giving it what each of those made, in that order.
/// The outcomes come back in the modules' order.
///
/// # Panics
///
/// If the imports form a cycle, which `graph` reports first.
pub fn run<T, E, F>(
    snapshot: &Snapshot,
    imports: &[Vec<usize>],
    job: F,
) -> Result<Vec<Outcome<T, E>>, Cancelled>
where
    T: Send + Sync,
    E: Send + Sync,
    F: Fn(usize, &[&T]) -> Result<T, E> + Sync,
{
    let n = imports.len();
    let mut importers = vec![Vec::new(); n];
    for (i, deps) in imports.iter().enumerate() {
        for &d in deps {
            importers[d].push(i);
        }
    }
    let run = Run {
        snapshot,
        imports,
        importers,
        waiting: imports.iter().map(|d| AtomicUsize::new(d.len())).collect(),
        outcomes: (0..n).map(|_| OnceLock::new()).collect(),
        cancelled: AtomicBool::new(false),
        job,
    };
    Pool::global().scope(|scope| {
        let run = &run;
        for (i, deps) in imports.iter().enumerate() {
            if deps.is_empty() {
                scope.spawn(move |scope| run.module(scope, i));
            }
        }
    });
    if run.cancelled.load(Ordering::Relaxed) {
        return Err(Cancelled);
    }
    let outcomes = run.outcomes.into_iter().map(|outcome| {
        outcome
            .into_inner()
            .expect("the imports form no cycle, so every module ran")
    });
    Ok(outcomes.collect())
}

struct Run<'a, T, E, F> {
    snapshot: &'a Snapshot,
    imports: &'a [Vec<usize>],
    importers: Vec<Vec<usize>>,
    /// How many of each module's imports haven't finished.
    waiting: Vec<AtomicUsize>,
    outcomes: Vec<OnceLock<Outcome<T, E>>>,
    cancelled: AtomicBool,
    job: F,
}

impl<'a, T, E, F> Run<'a, T, E, F>
where
    T: Send + Sync,
    E: Send + Sync,
    F: Fn(usize, &[&T]) -> Result<T, E> + Sync,
{
    fn module<'s>(&'s self, scope: &rayon::Scope<'s>, i: usize) {
        if self.cancelled.load(Ordering::Relaxed) || self.snapshot.is_stale() {
            self.cancelled.store(true, Ordering::Relaxed);
            return;
        }
        let made: Option<Vec<&T>> = self.imports[i]
            .iter()
            .map(|&d| match self.outcomes[d].get() {
                Some(Outcome::Done(made)) => Some(made),
                _ => None,
            })
            .collect();
        let outcome = match made {
            Some(made) => match (self.job)(i, &made) {
                Ok(made) => Outcome::Done(made),
                Err(e) => Outcome::Failed(e),
            },
            None => Outcome::Blocked,
        };
        if self.outcomes[i].set(outcome).is_err() {
            unreachable!("each module runs once");
        }
        for &importer in &self.importers[i] {
            // The last import to finish starts its importer, and the
            // release makes every import's outcome visible to it.
            if self.waiting[importer].fetch_sub(1, Ordering::AcqRel) == 1 {
                scope.spawn(move |scope| self.module(scope, importer));
            }
        }
    }
}
