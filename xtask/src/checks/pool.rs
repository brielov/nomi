//! The engine's pool is the process's only one (docs/architecture.md §4,
//! §6): no other nomi crate depends on a thread-pool crate itself.

/// The crate that owns the pool, by short name.
const OWNER: &str = "engine";

/// Crates that build thread pools.
const POOLS: &[&str] = &["rayon", "rayon-core", "threadpool", "tokio"];

/// `direct` pairs each nomi crate, by short name, with the external crates
/// it names as normal dependencies.
pub(crate) fn check(direct: &[(String, Vec<String>)]) -> Vec<String> {
    direct
        .iter()
        .filter(|(krate, _)| krate != OWNER)
        .flat_map(|(krate, deps)| {
            deps.iter()
                .filter(|dep| POOLS.contains(&dep.as_str()))
                .map(move |dep| {
                    format!("nomi-{krate} depends on `{dep}`; the engine's pool is the only one (docs/architecture.md §6)")
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(krate: &str, deps: &[&str]) -> (String, Vec<String>) {
        (
            krate.to_string(),
            deps.iter().map(|d| d.to_string()).collect(),
        )
    }

    #[test]
    fn the_engine_may_build_the_pool() {
        assert!(check(&[direct("engine", &["rayon", "walkdir"])]).is_empty());
    }

    #[test]
    fn another_crate_may_not() {
        let violations = check(&[direct("cli", &["clap", "rayon"])]);
        assert_eq!(
            violations,
            [
                "nomi-cli depends on `rayon`; the engine's pool is the only one (docs/architecture.md §6)"
            ]
        );
    }
}
