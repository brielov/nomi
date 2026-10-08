//! Elm's package registry: the website's protocol and the package index.
//! The solver and the package store join them in 3.5 and 3.7. Callers
//! say where files go; `nomi_engine::paths` computes those paths.

pub mod http;
pub mod index;

pub use http::{Http, HttpError, Web};
pub use index::{Connection, Index, Known, LatestError, RegistryProblem};
