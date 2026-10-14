//! The test runner, elm-test's behaviour over nomi's engine: its command
//! line, which files and tests a run takes, the runner it generates and
//! builds, the workers that run it, and the reports they make.

mod build;
mod command;
mod discover;
mod files;
mod generate;
mod install;
mod json;
mod junit;
mod runner;
mod supervise;
mod watch;

pub use build::{Env, Failure, Runner, Step, build};
pub use command::{Flags, Parsed, ReportName, Run, Setup, main, parse};
pub use discover::{TestModule, modules};
pub use files::{Project, resolve};
pub use generate::{Options, Report};
pub use supervise::{Ended, Supervise};

/// The package tests are written with.
const TEST_PACKAGE: &str = "elm-explorations/test";
