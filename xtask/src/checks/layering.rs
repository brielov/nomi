//! Crate edges must be a subset of the graph in architecture §4.

/// Allowed dependencies between nomi crates, by short name (`nomi-` stripped).
/// This table is the enforced form of docs/architecture.md §4; change both
/// together, with a DECISIONS.md entry.
pub const ALLOWED: &[(&str, &[&str])] = &[
    ("base", &[]),
    ("syntax", &["base"]),
    ("format", &["base", "syntax"]),
    ("compiler", &["base", "syntax"]),
    ("codegen-js", &["base", "compiler"]),
    ("project", &["base"]),
    ("registry", &["base", "project"]),
    (
        "engine",
        &[
            "base",
            "syntax",
            "format",
            "compiler",
            "codegen-js",
            "project",
            "registry",
        ],
    ),
    (
        "ide",
        &["base", "syntax", "format", "compiler", "project", "engine"],
    ),
    ("lsp", &["base", "ide", "engine"]),
    ("test", &["base", "syntax", "compiler", "project", "engine"]),
    (
        "cli",
        &[
            "base",
            "syntax",
            "format",
            "compiler",
            "codegen-js",
            "project",
            "registry",
            "engine",
            "ide",
            "lsp",
            "test",
        ],
    ),
    // Test support; any crate may use it, but only as a dev-dependency.
    ("testkit", &[]),
];

/// The crate every crate may use in its tests and nowhere else.
const TESTKIT: &str = "testkit";

/// A dependency of one nomi crate on another, by short name.
#[derive(Debug)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub dev: bool,
}

/// `crates` are the nomi crates in the workspace and `edges` their
/// dependencies on each other, all by short name.
pub fn check(crates: &[String], edges: &[Edge]) -> Vec<String> {
    let allowed = |name: &str| ALLOWED.iter().find(|(c, _)| *c == name).map(|(_, d)| *d);
    let mut violations = Vec::new();
    for name in crates {
        if allowed(name).is_none() {
            violations.push(format!(
                "nomi-{name} is not in the crate table (docs/architecture.md §4, xtask/src/checks/layering.rs)"
            ));
        }
    }
    for Edge { from, to, dev } in edges {
        if to == TESTKIT {
            if !dev {
                violations.push(format!(
                    "nomi-{from} may depend on nomi-testkit only as a dev-dependency"
                ));
            }
            continue;
        }
        if let Some(deps) = allowed(from)
            && !deps.contains(&to.as_str())
        {
            violations.push(format!(
                "nomi-{from} may not depend on nomi-{to} (docs/architecture.md §4)"
            ));
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            dev: false,
        }
    }

    fn dev_edge(from: &str, to: &str) -> Edge {
        Edge {
            dev: true,
            ..edge(from, to)
        }
    }

    #[test]
    fn allowed_edges_pass() {
        let crates = names(&["base", "syntax", "cli"]);
        let edges = [edge("syntax", "base"), edge("cli", "syntax")];
        assert!(check(&crates, &edges).is_empty());
    }

    #[test]
    fn upward_edge_fails() {
        let crates = names(&["base", "syntax", "format"]);
        let violations = check(&crates, &[edge("syntax", "format")]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("nomi-syntax may not depend on nomi-format"));
    }

    #[test]
    fn testkit_only_as_a_dev_dependency() {
        let crates = names(&["base", "syntax", "testkit"]);
        assert!(check(&crates, &[dev_edge("syntax", "testkit")]).is_empty());
        let violations = check(&crates, &[edge("syntax", "testkit")]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("only as a dev-dependency"));
    }

    #[test]
    fn dev_edges_follow_the_table_too() {
        let crates = names(&["base", "syntax", "format"]);
        assert_eq!(check(&crates, &[dev_edge("syntax", "format")]).len(), 1);
    }

    #[test]
    fn unknown_crate_fails() {
        let violations = check(&names(&["utils"]), &[]);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("nomi-utils"));
    }

    #[test]
    fn table_is_acyclic_and_closed() {
        for (name, deps) in ALLOWED {
            for dep in *deps {
                assert_ne!(name, dep);
                assert!(ALLOWED.iter().any(|(c, _)| c == dep), "{dep} unknown");
            }
        }
        // Every crate appears after its dependencies, which rules out cycles.
        for (i, (_, deps)) in ALLOWED.iter().enumerate() {
            for dep in *deps {
                let j = ALLOWED.iter().position(|(c, _)| c == dep).unwrap();
                assert!(j < i, "{dep} must be listed before its dependents");
            }
        }
    }
}
