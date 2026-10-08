//! Elm's package registry: the website's protocol, the package index and
//! the package store. Callers say where files go; `nomi_engine::paths`
//! computes those paths, and the engine's pool downloads in parallel.

pub mod http;
pub mod index;
pub mod store;

pub use http::{Http, HttpError, Web};
pub use index::{Connection, Index, Known, LatestError, RegistryProblem};
pub use store::{PackageProblem, Store};
