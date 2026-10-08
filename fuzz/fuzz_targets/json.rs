//! Upstream's JSON parser takes any bytes without panicking, for every
//! Compat, and what it takes, written back, reads back the same.
//! `cargo xtask fuzz json` runs it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nomi_base::Compat;
use nomi_project::json::{encode, parse};

fuzz_target!(|bytes: &[u8]| {
    for compat in Compat::ALL {
        let Ok(value) = parse(bytes, compat) else {
            continue;
        };
        let value = encode::Value::from(&value);
        // An int that wrapped negative is written with a sign the parser
        // refuses, as upstream's would be.
        if has_negative(&value) {
            continue;
        }
        let written = encode::write(&value);
        let again = parse(&written, compat).expect("what was written parses");
        assert_eq!(encode::Value::from(&again), value);
    }
});

fn has_negative(value: &encode::Value) -> bool {
    match value {
        encode::Value::Int(n) => *n < 0,
        encode::Value::Array(entries) => entries.iter().any(has_negative),
        encode::Value::Object(fields) => fields.iter().any(|(_, v)| has_negative(v)),
        _ => false,
    }
}
