//! Upstream's JSON parser takes any bytes without panicking, for every
//! Compat, and what it takes, written back, reads back the same.
//! `cargo xtask fuzz json` runs it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nomi_base::Compat;
use nomi_project::json::encode::{self, Value};
use nomi_project::json::parse;

fuzz_target!(|bytes: &[u8]| {
    for compat in Compat::ALL {
        let Ok(value) = parse(bytes, compat) else {
            continue;
        };
        let value = Value::from(&value);
        // An int that wrapped negative is written with a sign the parser
        // refuses, as upstream's would be.
        if has_negative(&value) {
            continue;
        }
        let written = encode::write(&value);
        let again = parse(&written, compat).expect("what was written parses");
        // Compared as written, which is guarded against deep nesting
        // where comparing the values isn't.
        assert_eq!(encode::write(&Value::from(&again)), written);
    }
});

/// Whether `value` holds a negative int, at any depth.
fn has_negative(value: &Value) -> bool {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Int(n) if *n < 0 => return true,
            Value::Array(entries) => pending.extend(entries),
            Value::Object(fields) => pending.extend(fields.iter().map(|(_, v)| v)),
            Value::Int(_) | Value::String(_) | Value::Bool(_) | Value::Null => {}
        }
    }
    false
}
