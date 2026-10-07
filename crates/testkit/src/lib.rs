//! Test support shared by every crate's tests and by xtask: parity
//! ratchets, their expected results, and source mutations. Only ever a
//! dev-dependency of nomi crates; `xtask ci` enforces that.

pub mod mutation;
pub mod ratchet;
