//! The test runner (plan M10), elm-test's behaviour over nomi's engine:
//! which files and tests a run takes, and later the runner it generates,
//! the workers that run it and the reports they make.

pub mod discover;
pub mod files;

pub use discover::{TestModule, modules, tests};
pub use files::{Project, resolve};
