//! The lexical scan of an FFI module's exports takes any text without
//! panicking, and each name it finds is spelt in the text, as the scan
//! takes names and string literals as they are written.
//! `cargo xtask fuzz ffi` runs it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nomi_codegen_js::ffi::exports;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(source) = std::str::from_utf8(bytes) {
        for name in exports(source).names {
            assert!(source.contains(&name), "{name:?}");
        }
    }
});
