//! Test support shared by every crate's tests: parity ratchets, their
//! expected results, and source mutations. Only ever a dev-dependency;
//! `xtask ci` enforces that.

pub mod mutation;
pub mod ratchet;
