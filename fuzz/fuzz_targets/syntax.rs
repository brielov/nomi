//! Every entry point keeps [`nomi_syntax::check_invariants`] for any
//! text. `cargo xtask fuzz syntax` runs it; so does `cargo fuzz run
//! syntax`, on nightly.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes) {
        nomi_syntax::check_invariants(text);
    }
});
