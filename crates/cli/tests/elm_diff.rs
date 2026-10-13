//! nomi invoked as `elm`, against each elm's transcripts of `elm diff` and `elm bump`, which `cargo xtask oracle diff`
//! recorded online. `support/registry.rs` runs them against a fixture
//! registry served on the loopback interface.
#![cfg(unix)]

#[path = "support/registry.rs"]
mod registry;
#[path = "support/server.rs"]
mod server;
#[path = "support/website.rs"]
mod website;

#[test]
fn elm_diff() {
    registry::transcripts("tests/elm_diff");
}
