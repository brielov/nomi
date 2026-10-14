//! The documentation ratchet: each documentation project as each
//! release, passing when nomi's `--report=json`, terminal report and
//! `docs.json` are elm's, whole.
#![cfg(unix)]

#[path = "support/projects.rs"]
pub mod projects;

#[path = "support/names.rs"]
pub mod names;

use nomi_engine::check::Goal;

#[test]
fn docs_reports() {
    names::reports_ratchet("docs", Goal::Docs);
}
