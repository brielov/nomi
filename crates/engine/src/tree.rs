//! `nomi tree`, which upstream has no command for: the releases the
//! project builds with, under the dependencies that need them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use nomi_project::Name;
use nomi_registry::{Details, Http, details};

use crate::console::{Console, Personality};
use crate::install::details_problem;
use crate::{Context, env};

/// Prints the dependency tree of the project around the directory.
/// Whether it succeeded.
pub fn run(cx: &Context<'_, impl Http>, console: &mut dyn Console) -> bool {
    env::with_registry(
        cx,
        console,
        "to show",
        "I need the list of published packages to know what your dependencies need",
        |console, project, registry| {
            let solution = details::verify(registry, project.compat, &project.outline)
                .map_err(|problem| details_problem(&problem, project.compat, Personality::Nomi))?;
            let direct: Vec<&Name> = project.outline.direct().collect();
            let test: Vec<&Name> = project.outline.test_direct().collect();
            console.out(&render(&direct, &test, &solution));
            Ok(true)
        },
    )
}

/// The trees under `direct` and then `test`, each release's dependencies
/// shown once and marked `(*)` after that.
fn render(direct: &[&Name], test: &[&Name], solution: &BTreeMap<Name, Details>) -> String {
    let mut tree = Tree {
        solution,
        shown: BTreeSet::new(),
        out: String::new(),
    };
    for name in direct {
        tree.node(name, "", "");
    }
    if !test.is_empty() {
        if !direct.is_empty() {
            tree.out.push('\n');
        }
        tree.out.push_str("[test-dependencies]\n");
        for name in test {
            tree.node(name, "", "");
        }
    }
    tree.out
}

struct Tree<'a> {
    solution: &'a BTreeMap<Name, Details>,
    shown: BTreeSet<&'a Name>,
    out: String,
}

impl<'a> Tree<'a> {
    /// `name` after `lead`, and its dependencies under it after `indent`.
    fn node(&mut self, name: &Name, lead: &str, indent: &str) {
        let Some((name, details)) = self.solution.get_key_value(name) else {
            return;
        };
        let deps: Vec<&Name> = details
            .deps
            .keys()
            .filter(|dep| self.solution.contains_key(*dep))
            .collect();
        let repeated = !deps.is_empty() && !self.shown.insert(name);
        let mark = if repeated { " (*)" } else { "" };
        let _ = writeln!(self.out, "{lead}{name} {}{mark}", details.version);
        if repeated {
            return;
        }
        for (i, dep) in deps.iter().enumerate() {
            let last = i + 1 == deps.len();
            let (lead, more) = if last {
                ("└── ", "    ")
            } else {
                ("├── ", "│   ")
            };
            self.node(dep, &format!("{indent}{lead}"), &format!("{indent}{more}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use nomi_base::Compat;
    use nomi_project::{Constraint, Version};

    use super::*;

    #[test]
    fn each_tree_shows_once() {
        let compat = Compat::LATEST;
        let name = |s: &str| s.parse::<Name>().unwrap();
        let release = |deps: &[&str]| Details {
            version: Version::new(1, 0, 0),
            deps: deps
                .iter()
                .map(|dep| (name(dep), Constraint::anything(compat)))
                .collect(),
        };
        let solution = BTreeMap::from([
            (
                name("elm/browser"),
                release(&["elm/core", "elm/html", "elm/json"]),
            ),
            (name("elm/core"), release(&[])),
            (name("elm/html"), release(&["elm/core", "elm/json"])),
            (name("elm/json"), release(&["elm/core"])),
            (name("elm/test"), release(&["elm/core", "elm/html"])),
        ]);
        let browser = name("elm/browser");
        let html = name("elm/html");
        let test = name("elm/test");
        assert_eq!(
            render(&[&browser, &html], &[&test], &solution),
            "\
elm/browser 1.0.0
├── elm/core 1.0.0
├── elm/html 1.0.0
│   ├── elm/core 1.0.0
│   └── elm/json 1.0.0
│       └── elm/core 1.0.0
└── elm/json 1.0.0 (*)
elm/html 1.0.0 (*)

[test-dependencies]
elm/test 1.0.0
├── elm/core 1.0.0
└── elm/html 1.0.0 (*)
"
        );
    }
}
