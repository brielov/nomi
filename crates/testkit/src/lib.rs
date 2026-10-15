//! Test support shared by every crate's tests and by xtask: parity
//! ratchets, their expected results, source mutations, command-line
//! transcripts and cases made by editing them, the fixture packages, the
//! solver scenarios, benchmark timing, and directory trees. Only ever a
//! dev-dependency of nomi crates; `xtask ci` enforces that.

pub mod bench;
pub mod edit;
pub mod lsp;
pub mod mutation;
pub mod packages;
pub mod ratchet;
pub mod solver;
pub mod transcript;
mod tree;

pub use tree::{copy_tree, files_below, padded_dir, tree_digest};

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

/// Fails the test unless Node runs: the tests that run generated code
/// need it, and never pass by skipping.
pub fn require_node() {
    let runs = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(runs, "this test runs Node, which isn't on the PATH");
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
