//! The checks after type inference (`Nitpick/*.hs`): patterns that miss
//! cases or can never match, and `Debug` uses `--optimize` refuses.

pub mod debug;
pub mod main;
pub mod patterns;
pub mod report;
