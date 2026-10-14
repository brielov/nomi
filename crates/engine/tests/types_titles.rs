//! The first type ratchet: each type-error project as each release,
//! passing when nomi reports the problems elm does, by title and region,
//! in each module elm names.
#![cfg(unix)]

#[path = "support/projects.rs"]
pub mod projects;

#[path = "support/names.rs"]
pub mod names;

#[test]
fn types_titles() {
    names::titles_ratchet("types");
}
