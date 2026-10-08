//! Formatting keeps [`nomi_format::check_invariants`] for any text. `cargo
//! xtask fuzz format` runs it; so does `cargo fuzz run format`, on nightly.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes) {
        nomi_format::check_invariants(text);
    }
});
