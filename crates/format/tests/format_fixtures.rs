//! The fixture side of the format ratchet: each file under
//! `format_fixtures/inputs/` is a case, passing when nomi formats it to
//! the file of the same name under `expected/`, or rejects it where
//! `rejected.tsv` says elm-format did, and keeping the formatter's
//! invariants but where `invariants.tsv` says elm-format breaks one.
//! `cargo xtask oracle format fixtures` stores the first two. Where nomi
//! deviates from elm-format on purpose, `deviations/` holds its output
//! instead, by hand, and the case keeps every invariant; each deviation
//! has its entry in `DECISIONS.md`.

use std::collections::BTreeMap;
use std::path::Path;

use nomi_testkit::ratchet::Suite;

const RERUN: &str = "rerun `cargo xtask oracle format fixtures`";

#[test]
fn format_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/format_fixtures");
    let table = std::fs::read_to_string(dir.join("rejected.tsv"))
        .unwrap_or_else(|e| panic!("reading rejected.tsv: {e}; {RERUN}"));
    // The case, and where elm-format stopped or `crash`.
    let rejected = rows(&table);
    let table = std::fs::read_to_string(dir.join("invariants.tsv")).unwrap();
    // The case, and the invariants elm-format breaks there.
    let broken = rows(&table);
    let suite = Suite::open(&dir);
    let cases = inputs(&dir.join("inputs"));
    for deviation in inputs(&dir.join("deviations")) {
        assert!(
            cases.contains(&deviation),
            "deviations/{deviation}: no input"
        );
    }
    let mut outcomes = Vec::new();
    for case in cases {
        let text = std::fs::read(dir.join("inputs").join(&case)).unwrap();
        let text = std::str::from_utf8(&text).ok();
        let formatted = text.and_then(nomi_format::format);
        let result = match (rejected.get(case.as_str()), formatted) {
            // elm-format crashes here; nomi may do as it sees fit.
            (Some(&"crash"), Some(_)) => Ok(()),
            (Some(&"crash"), None) => Err("elm-format crashes, nomi rejects".to_owned()),
            (Some(_), None) => Ok(()),
            (Some(at), Some(_)) => Err(format!("elm-format rejects at {at}, nomi formats")),
            (None, formatted) if dir.join("deviations").join(&case).exists() => {
                let deviation = std::fs::read(dir.join("deviations").join(&case)).unwrap();
                match formatted {
                    Some(output) if output.as_bytes() == deviation => Ok(()),
                    _ => Err("nomi's output differs from its deviation".to_owned()),
                }
            }
            (None, formatted) => {
                let expected = suite
                    .expected(&case)
                    .unwrap_or_else(|| panic!("{case}: no expected output; {RERUN}"));
                match formatted {
                    None => Err("elm-format formats, nomi rejects".to_owned()),
                    Some(output) if expected.matches(output.as_bytes()) => Ok(()),
                    Some(_) => Err("nomi's output differs from elm-format's".to_owned()),
                }
            }
        };
        let result = result.and_then(|()| {
            let found = text.map_or_else(Vec::new, nomi_format::broken_invariants);
            let names: Vec<_> = found.iter().map(|(i, _)| i.name()).collect();
            let listed = broken.get(case.as_str()).copied().unwrap_or("");
            match found.first() {
                _ if names.join(",") == listed => Ok(()),
                Some((invariant, how)) if listed.is_empty() => {
                    Err(format!("breaks `{}`: {how}", invariant.name()))
                }
                _ => Err(format!(
                    "breaks `{}`, where invariants.tsv lists `{listed}`",
                    names.join(",")
                )),
            }
        });
        outcomes.push((case, result));
    }
    suite.finish(outcomes);
}

/// A table's rows: a case, a tab, and a cell.
fn rows(table: &str) -> BTreeMap<&str, &str> {
    table
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            l.split_once('\t')
                .unwrap_or_else(|| panic!("malformed row `{l}`"))
        })
        .collect()
}

/// Every file below `dir`, as `/`-separated relative paths, sorted.
fn inputs(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap();
                let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
                found.push(parts.join("/"));
            }
        }
    }
    found.sort();
    found
}
