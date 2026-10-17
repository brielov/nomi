//! A `nomi.toml` of any bytes is read or refused without panicking, and
//! every span it gives, of a member or a problem, lies within it.
//! `cargo xtask fuzz manifest` runs it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nomi_project::Manifest;

fuzz_target!(|bytes: &[u8]| {
    let within = |range: nomi_base::TextRange| usize::from(range.end()) <= bytes.len();
    match Manifest::parse(bytes) {
        Ok(manifest) => {
            for member in manifest.workspace.iter().flat_map(|w| &w.members) {
                assert!(
                    within(member.range),
                    "{:?} of {}",
                    member.range,
                    bytes.len()
                );
            }
        }
        Err(error) => assert!(within(error.range), "{error:?} of {}", bytes.len()),
    }
});
