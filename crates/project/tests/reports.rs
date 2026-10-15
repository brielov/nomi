//! Every report elm wrote about an `elm.json` fixture under
//! `outline_fixtures/`, as `cargo xtask oracle outline` stored it in
//! `reports.tsv`, against nomi's for the same problem, byte for byte as
//! `--report=json` writes it. The JSON holds the message laid out as the
//! terminal shows it, colors and all.

use std::path::Path;

use nomi_base::diagnostic::json;
use nomi_project::Outline;
use nomi_project::report::{self, ElmCommand};
use nomi_testkit::release_column;

const RERUN: &str = "rerun `cargo xtask oracle outline`";

#[test]
fn reports() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/outline_fixtures");
    let table = std::fs::read_to_string(dir.join("reports.tsv"))
        .unwrap_or_else(|e| panic!("reading reports.tsv: {e}; {RERUN}"));
    let mut failures = Vec::new();
    let mut checked = 0;
    for line in table.lines().filter(|l| !l.starts_with('#')).skip(1) {
        // The report is last, so a tab elm copied into it stays in it.
        let [name, elms, expected] = line.splitn(3, '\t').collect::<Vec<_>>()[..] else {
            panic!("a bad row: {line}");
        };
        let src = std::fs::read(dir.join(format!("{name}.json")))
            .unwrap_or_else(|e| panic!("{name}: {e}; {RERUN}"));
        for elm in elms.split(',') {
            let compat = release_column(elm).unwrap_or_else(|| panic!("unknown elm {elm}"));
            let found = match Outline::read(&src, compat) {
                Ok(_) => "accepted".to_owned(),
                Err(e) => {
                    let command = |c: ElmCommand| format!("elm {}", c.elm_name());
                    let report = report::outline(&e, &src, compat, &command);
                    json::report(&report, compat).to_string()
                }
            };
            checked += 1;
            if found != expected {
                failures.push(format!(
                    "{name} on {compat}:\n  nomi {found}\n  elm  {expected}"
                ));
            }
        }
    }
    assert!(checked > 0, "no reports; {RERUN}");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
