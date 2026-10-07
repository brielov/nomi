//! A toy suite that runs the ratchet end to end: each case's output is its
//! input in upper case. `pending` never matches, so it stays unlisted.

use std::path::Path;

use nomi_testkit::ratchet::Suite;

#[test]
fn toy() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/toy");
    let suite = Suite::open(&dir);
    let mut cases: Vec<_> = std::fs::read_dir(dir.join("cases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    cases.sort();
    let outcomes: Vec<_> = cases
        .into_iter()
        .map(|case| {
            let input = std::fs::read_to_string(dir.join("cases").join(&case)).unwrap();
            let actual = input.to_uppercase();
            let result = match suite.expected(&case) {
                Some(expected) if expected.matches(actual.as_bytes()) => Ok(()),
                Some(_) => Err(format!("output differs: {actual:?}")),
                None => Err("no expected result".to_string()),
            };
            (case, result)
        })
        .collect();
    suite.finish(outcomes);
}
