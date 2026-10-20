//! Test support shared by every crate's tests and by xtask: parity
//! ratchets, their expected results, source mutations, command-line
//! transcripts and cases made by editing them, waiting for a child within
//! a limit, the fixture packages, the solver scenarios, benchmark timing,
//! directory trees, and workspace members' `elm.json` files. Only ever a
//! dev-dependency of nomi crates; `xtask ci` enforces that.

pub mod bench;
pub mod edit;
pub mod http;
pub mod lsp;
pub mod mutation;
pub mod packages;
pub mod process;
pub mod ratchet;
pub mod solver;
pub mod transcript;
mod tree;
pub mod workspace;

pub use tree::{copy_tree, files_below, padded_dir, tree_digest, write_file};

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use nomi_base::Compat;
use sha2::{Digest as _, Sha256};

/// The SHA-256 of `bytes`, in lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    nomi_base::hex(&Sha256::digest(bytes))
}

/// What `compat`'s elm wrote for the optimizer cases, under
/// `crates/compiler/tests/optimized/artifacts/elm-<release>/`: each
/// case's modules, and the fixture packages' `o.dat`.
pub fn optimized_artifacts(compat: Compat) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../compiler/tests/optimized/artifacts")
        .join(format!("elm-{compat}"))
}

/// The release an oracle table's column names, as `elm 0.19.1` does.
pub fn release_column(label: &str) -> Option<Compat> {
    label.strip_prefix("elm ")?.parse().ok()
}

/// Runs `f` over `items` on `workers` threads, each taking the next item
/// as it finishes one, and gives back the results in order.
pub fn parallel_on<T: Sync, R: Send>(
    workers: usize,
    items: &[T],
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = f(item);
                    nomi_base::lock(&results)[i] = Some(result);
                }
            });
        }
    });
    (results.into_inner())
        .expect("no worker panicked")
        .into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

/// Fails the test unless Bun runs: the tests that run generated code
/// need it, and never pass by skipping.
pub fn require_bun() {
    let runs = Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(runs, "this test runs Bun, which isn't on the PATH");
}

/// The Bun binary the tests that run programs use: the one
/// `NOMI_TEST_BUN` names, to test another release, or else `bun` on the
/// PATH. It is the binary Bun reports running, since a version manager's
/// wrapper may exit 128 plus a signal itself rather than pass it on.
pub fn bun() -> PathBuf {
    let launcher = std::env::var_os("NOMI_TEST_BUN").unwrap_or_else(|| "bun".into());
    let output = Command::new(&launcher)
        .args(["-p", "process.execPath"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        }
        _ => panic!("this test runs Bun, and {launcher:?} doesn't run"),
    }
}

/// The PATH with the directory of [`bun`] first, so a program run under
/// `bun` runs on the tests' Bun.
pub fn path_with_bun() -> std::ffi::OsString {
    let bun = bun();
    let mut path = std::ffi::OsString::from(bun.parent().expect("bun is in a directory"));
    path.push(if cfg!(windows) { ";" } else { ":" });
    path.push(std::env::var_os("PATH").unwrap_or_default());
    path
}

#[cfg(test)]
mod tests {
    use super::parallel_on;

    #[test]
    fn parallel_on_keeps_order() {
        let items: Vec<u32> = (0..100).collect();
        assert_eq!(
            parallel_on(8, &items, |x| x * 2),
            items.iter().map(|x| x * 2).collect::<Vec<_>>()
        );
    }
}
