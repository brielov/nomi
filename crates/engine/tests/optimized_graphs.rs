//! The optimizer ratchet (plan 8.1b): each module of each optimizer case
//! (`crates/compiler/tests/optimized/cases`) as each release, passing
//! when nomi's graph encodes to the bytes of the `.elmo` that elm wrote
//! (`cargo xtask oracle optimized`).
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

use std::fs;
use std::path::Path;

use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::Case;

#[test]
fn optimized_graphs() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = manifest.join("../compiler/tests/optimized");
    let packages = manifest.join("../compiler/tests/packages");
    let default = fs::read_to_string(cases.join("default.elm.json")).unwrap();
    let mut names: Vec<_> = fs::read_dir(cases.join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    let mut outcomes = Vec::new();
    for path in &names {
        let case_name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let case = Case::parse(&fs::read_to_string(path).unwrap()).unwrap();
        for compat in projects::ALL {
            let elm = cases
                .join("artifacts")
                .join(format!("elm-{compat}"))
                .join(&case_name);
            let mut expected: Vec<_> = fs::read_dir(&elm)
                .unwrap_or_else(|e| {
                    panic!("{}: {e}; run `cargo xtask oracle optimized`", elm.display())
                })
                .map(|e| e.unwrap().path())
                .collect();
            expected.sort();
            let made = projects::optimized(&case, compat, &default, &packages);
            for file in expected {
                let module = file.file_stem().unwrap().to_string_lossy().into_owned();
                let id = format!("{case_name} {module} as {compat}");
                let outcome = match &made {
                    Err(why) => Err(format!("nomi rejected the case:\n{why}")),
                    Ok(graphs) => match graphs.iter().find(|(name, _)| *name == module) {
                        None => Err("nomi made no graph".to_owned()),
                        Some((_, bytes)) if *bytes == fs::read(&file).unwrap() => Ok(()),
                        Some(_) => Err("the graphs differ".to_owned()),
                    },
                };
                if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
                    eprintln!("FAIL {id}: {why}");
                }
                outcomes.push((id, outcome));
            }
        }
    }
    Suite::open(manifest.join("tests/optimized_graphs")).finish(outcomes);
}
