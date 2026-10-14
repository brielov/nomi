//! The optimizer ratchet: each module of each optimizer case
//! (`crates/compiler/tests/optimized/cases`) as each release, passing
//! when nomi's graph encodes to the bytes of the `.elmo` that elm wrote
//! (`cargo xtask oracle optimized`).
#![cfg(unix)]

#[path = "support/projects.rs"]
pub mod projects;

use std::fs;
use std::path::Path;

use nomi_base::Compat;
use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::Case;

#[test]
fn optimized_graphs() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = manifest.join("../compiler/tests/optimized");
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
        for compat in Compat::ALL {
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
            let made = projects::optimized(&case, compat, &default);
            for file in expected {
                let module = file.file_stem().unwrap().to_string_lossy().into_owned();
                let id = format!("{case_name} {module} as {compat}");
                let outcome = match &made {
                    Err(why) => Err(format!("nomi rejected the case:\n{why}")),
                    Ok(graphs) => match graphs.iter().find(|(name, _)| *name == module) {
                        None => Err("nomi made no graph".to_owned()),
                        Some((_, bytes)) if *bytes == fs::read(&file).unwrap() => Ok(()),
                        Some((_, bytes)) => {
                            Err(difference(bytes, &fs::read(&file).unwrap(), compat))
                        }
                    },
                };
                outcomes.push(projects::shown(id, outcome));
            }
        }
    }
    Suite::open(manifest.join("tests/optimized_graphs")).finish(outcomes);
}

/// Where nomi's graph first differs from elm's.
fn difference(nomi: &[u8], elm: &[u8], compat: Compat) -> String {
    use nomi_compiler::optimized::binary::decode_local_graph;
    let (Some(nomi), Some(elm)) = (
        decode_local_graph(nomi, compat),
        decode_local_graph(elm, compat),
    ) else {
        return "a graph doesn't decode".to_owned();
    };
    if nomi.main != elm.main {
        return format!("main:\nnomi: {:?}\nelm:  {:?}", nomi.main, elm.main);
    }
    for (global, node) in &elm.nodes {
        match nomi.nodes.get(global) {
            None => return format!("nomi has no {}", global.name),
            Some(n) if n != node => {
                let (a, b) = (format!("{n:?}"), format!("{node:?}"));
                let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
                let from = at.saturating_sub(100);
                let end = |s: &str| s.len().min(at + 200);
                return format!(
                    "{} differs at byte {at}:\nnomi: …{}\nelm:  …{}",
                    global.name,
                    &a[from..end(&a)],
                    &b[from..end(&b)]
                );
            }
            Some(_) => {}
        }
    }
    if let Some(global) = nomi.nodes.keys().find(|g| !elm.nodes.contains_key(*g)) {
        return format!("elm has no {}", global.name);
    }
    if nomi.fields != elm.fields {
        return format!("fields:\nnomi: {:?}\nelm:  {:?}", nomi.fields, elm.fields);
    }
    "the same graph, encoded otherwise".to_owned()
}
