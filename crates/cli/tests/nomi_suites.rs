//! nomi/core's pure and stream modules, by elm-test suites `nomi test`
//! runs, each a project under `nomi_suites/`:
//!
//! - `bytes`: `Nomi.Bytes`, each function against elm/bytes' encoders
//!   and decoders on generated input, UTF-8 read strictly, refusing each
//!   class of bad sequence, and base64 and hex round-tripping.
//! - `stream`: `Nomi.Stream` and `Nomi.Stdio` on nomi/test's virtual
//!   clock: a slow writer holds back its producer, an interrupted read
//!   loses nothing, lines split across chunks, a line reader leaves the
//!   rest of the stream, reads are answered in order whether a task or the
//!   kernel makes the reader, and the fakes' stdio.
//! - `fs`: `Nomi.Fs` in memory on the virtual clock (watches, file times,
//!   streams, temporary directories) and `Nomi.Path`.
//! - `process`: `Nomi.Process` under nomi/test: no command runs unless
//!   `withCommands` answers it, each run starts with none run, and a
//!   fiber waiting for a signal leaves the clock free.
//! - `http`: a server and its clients on `Nomi.Net.loopback` under nomi
//!   test, a handler's sleep and a request's timeout on the test's clock.
//! - `crypto`: hashes and HMACs against published vectors, randomness
//!   seeded per test, argon2id and bcrypt, and compression whole and
//!   streamed, corrupt input refused.
//! - `web`: nomi/http's routes, cookies (RFC 6265's examples, signed and
//!   tampered) and forms (URL-encoded, and multipart with a part streamed).
//! - `log`: `Nomi.Log`'s lines, JSON or text on a terminal, with each
//!   logger's fields, which concurrent fibers never mix.

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;

use website::offline;

/// Runs the suite `name` with a fixed seed, which must pass.
fn passes(name: &str) {
    nomi_testkit::require_bun();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/nomi_suites")
        .join(name);
    nomi_testkit::copy_tree(&suite, &root, |_| true).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let cache = website::fixture_cache();
    let tested = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args(["test", "--offline", "--no-color"])
        .args(["--seed", "1", "--fuzz", "500"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&tested.stdout);
    assert_eq!(tested.status.code(), Some(0), "{out}{tested:?}");
    assert!(out.contains("Failed:   0\n"), "{out}");
}

#[test]
fn the_bytes_suite_passes() {
    passes("bytes");
}

#[test]
fn the_stream_suite_passes() {
    passes("stream");
}

#[test]
fn the_fs_suite_passes() {
    passes("fs");
}

#[test]
fn the_process_suite_passes() {
    passes("process");
}

#[test]
fn the_http_suite_passes() {
    passes("http");
}

#[test]
fn the_crypto_suite_passes() {
    passes("crypto");
}

#[test]
fn the_web_suite_passes() {
    passes("web");
}

#[test]
fn the_log_suite_passes() {
    passes("log");
}
