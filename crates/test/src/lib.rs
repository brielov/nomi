//! The test runner (plan M10), elm-test's behaviour over nomi's engine:
//! which files and tests a run takes, the runner it generates and builds,
//! and later the workers that run it and the reports they make.

pub mod build;
pub mod discover;
pub mod files;
pub mod generate;
pub mod junit;
pub mod runner;
pub mod supervise;

pub use build::{Failure, Runner, build};
pub use discover::{TestModule, modules, tests};
pub use files::{Project, resolve};
pub use generate::{Options, Report};
