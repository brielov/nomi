//! Test support shared by every crate's tests and by xtask: parity
//! ratchets, their expected results, source mutations, command-line
//! transcripts and cases made by editing them, the fixture packages,
//! benchmark timing, and directory trees. Only ever a dev-dependency of
//! nomi crates; `xtask ci` enforces that.

pub mod bench;
pub mod edit;
pub mod lsp;
pub mod mutation;
pub mod packages;
pub mod ratchet;
pub mod transcript;
mod tree;

pub use tree::{copy_tree, files_below, padded_dir, tree_digest};

use sha2::{Digest as _, Sha256};

/// The SHA-256 of `bytes`, in lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    nomi_base::hex(&Sha256::digest(bytes))
}
