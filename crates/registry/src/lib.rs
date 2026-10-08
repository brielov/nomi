//! Elm's package registry: the website's protocol, the package index, the
//! package store, and the solver over them. Callers say where files go;
//! `nomi_engine::paths` computes those paths, and the engine's pool
//! downloads in parallel.

pub mod details;
pub mod http;
pub mod index;
pub mod install;
pub mod registry;
pub mod remove;
pub mod report;
pub mod solver;
pub mod store;

pub use details::DetailsProblem;
pub use http::{Http, HttpError, Web};
pub use index::{Connection, Index, Known, LatestError, RegistryProblem};
pub use install::{Change, InstallProblem, Plan};
pub use registry::{Registry, SolverProblem};
pub use remove::RemoveProblem;
pub use solver::{AppSolution, Details, Packages, Release};
pub use store::{PackageProblem, Store};
