//! Elm's package registry: the website's protocol, the package index, the
//! package store, and the solver over them. Callers say where files go;
//! `nomi_engine::paths` computes those paths, and the engine's pool
//! downloads in parallel.

pub mod details;
mod env;
mod files;
pub mod http;
mod index;
pub mod install;
pub mod remove;
pub mod report;
pub mod solver;
pub mod store;
pub mod update;

#[cfg(test)]
mod fixture;

pub use details::DetailsProblem;
pub use env::{Registry, SolverProblem};
pub use http::{Http, HttpError, Part, Web};
pub use index::{Connection, Index, Known, LatestError, RegistryProblem};
pub use install::{Change, InstallProblem, Plan};
pub use remove::RemoveProblem;
pub use solver::{AppSolution, Details, Packages, Release};
pub use store::{PackageProblem, Store};
pub use update::{Outdated, UpdateProblem};
