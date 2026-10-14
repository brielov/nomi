//! The pattern ratchet: each pattern-error project as each release,
//! passing when nomi's `--report=json` and terminal report are elm's,
//! whole.
#![cfg(unix)]

#[path = "support/projects.rs"]
pub mod projects;

#[path = "support/names.rs"]
pub mod names;

use nomi_engine::check::Goal;

#[test]
fn patterns_reports() {
    names::reports_ratchet("patterns", Goal::Check);
}
