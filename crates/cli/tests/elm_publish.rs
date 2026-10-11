//! nomi invoked as `elm`, against each elm's transcripts of `elm publish` failing (plan 8.14c), which `cargo xtask oracle publish`
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
fn elm_publish() {
    registry::transcripts("tests/elm_publish");
}
