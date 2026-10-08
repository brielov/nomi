//! Pure crates (rule R2) touch no filesystem, network, process, thread,
//! environment or clock, and depend only on crates known to do none of that.

/// Crates that take values and return values (docs/architecture.md R2).
pub const PURE: &[&str] = &[
    "base",
    "syntax",
    "format",
    "compiler",
    "codegen-js",
    "project",
];

/// External crates a pure crate may reach through normal dependencies,
/// directly or transitively. Before adding one, check that it does no I/O,
/// spawns no threads and reads no clock or environment.
pub const ALLOWED_EXTERNAL: &[&str] = &[
    // rowan and its dependencies
    "rowan",
    "countme",
    "hashbrown",
    "memoffset",
    "rustc-hash",
    "text-size",
    "autocfg",
    // deep recursion
    "stacker",
    "psm",
    "cfg-if",
    "libc",
    "windows-sys",
    "windows-targets",
    "windows-link",
    // collections and small types
    "indexmap",
    "equivalent",
    "smallvec",
    "smol_str",
    "foldhash",
    "allocator-api2",
    // serialization and hashing
    "serde",
    "serde_core",
    "serde_derive",
    "proc-macro2",
    "quote",
    "syn",
    "unicode-ident",
    "serde_json",
    "itoa",
    "ryu",
    "memchr",
    "sha1",
    "sha2",
    "digest",
    "block-buffer",
    "crypto-common",
    "generic-array",
    "typenum",
    "version_check",
    "cpufeatures",
    // Unicode general categories and case mappings, pinned to 15.1
    "icu_properties",
    "icu_properties_data",
    "icu_casemap",
    "icu_casemap_data",
    "icu_collections",
    "icu_provider",
    "icu_provider_macros",
    "icu_locid",
    "icu_locid_transform",
    "icu_locid_transform_data",
    "displaydoc",
    "litemap",
    "stable_deref_trait",
    "synstructure",
    "tinystr",
    "writeable",
    "yoke",
    "yoke-derive",
    "zerofrom",
    "zerofrom-derive",
    "zerovec",
    "zerovec-derive",
    // exact float digits, as GHC's `floatToDigits` finds them
    "num-bigint",
    "num-integer",
    "num-traits",
];

/// Standard-library modules a pure crate may not name.
const FORBIDDEN_STD_MODULES: &[&str] = &["fs", "net", "process", "thread", "env", "os"];

/// Tokens that mean I/O or a clock even without a `std::` path.
const FORBIDDEN_TOKENS: &[&str] = &[
    "println!",
    "print!",
    "eprintln!",
    "eprint!",
    "dbg!",
    "stdin",
    "stdout",
    "stderr",
    "SystemTime",
    "UNIX_EPOCH",
    "Instant",
];

/// `closures` pairs each pure crate with the external crates its normal
/// dependencies reach.
pub fn check_dependencies(closures: &[(String, Vec<String>)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (krate, deps) in closures {
        for dep in deps {
            if !ALLOWED_EXTERNAL.contains(&dep.as_str()) {
                violations.push(format!(
                    "pure crate nomi-{krate} reaches `{dep}`, which is not in ALLOWED_EXTERNAL (xtask/src/checks/purity.rs)"
                ));
            }
        }
    }
    violations
}

/// Scans one source file of a pure crate. `label` names it in messages.
pub fn scan_source(label: &str, source: &str) -> Vec<String> {
    let code = strip_comments_and_strings(source);
    let mut violations = Vec::new();
    let mut report = |offset: usize, what: &str| {
        let line = code[..offset].matches('\n').count() + 1;
        violations.push(format!(
            "{label}:{line}: pure crates may not use {what} (rule R2)"
        ));
    };
    for (offset, module) in std_modules(&code) {
        if FORBIDDEN_STD_MODULES.contains(&module.as_str()) {
            report(offset, &format!("std::{module}"));
        }
    }
    for token in FORBIDDEN_TOKENS {
        for offset in token_offsets(&code, token) {
            report(offset, &format!("`{token}`"));
        }
    }
    violations
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Offsets of `token` where it is not part of a longer identifier.
fn token_offsets(code: &str, token: &str) -> Vec<usize> {
    let ends_in_ident = token.chars().last().is_some_and(is_ident);
    code.match_indices(token)
        .filter(|(i, _)| {
            let before = code[..*i].chars().next_back();
            let after = code[i + token.len()..].chars().next();
            !before.is_some_and(is_ident) && !(ends_in_ident && after.is_some_and(is_ident))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Every module named right after `std::`, including each item of a
/// `std::{a, b::c}` group.
fn std_modules(code: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for offset in token_offsets(code, "std") {
        let rest = code[offset + 3..].trim_start();
        let Some(rest) = rest.strip_prefix("::") else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(group) = rest.strip_prefix('{') {
            let mut depth = 0usize;
            let mut item_start = true;
            for (i, c) in group.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' if depth == 0 => break,
                    '}' => depth -= 1,
                    ',' if depth == 0 => item_start = true,
                    c if item_start && is_ident(c) => {
                        item_start = false;
                        let name: String =
                            group[i..].chars().take_while(|&c| is_ident(c)).collect();
                        found.push((offset, name));
                    }
                    _ => {}
                }
            }
        } else {
            let name: String = rest.chars().take_while(|&c| is_ident(c)).collect();
            if !name.is_empty() {
                found.push((offset, name));
            }
        }
    }
    found
}

/// Replaces comments and the contents of string and char literals with
/// spaces, keeping newlines so offsets still map to the right lines.
pub fn strip_comments_and_strings(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == 'r'
            && matches!(next, Some('"' | '#'))
            && !out.chars().next_back().is_some_and(is_ident)
        {
            // Raw string: r"..." or r#"..."#.
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) != Some(&'"') {
                out.push(c);
                i += 1;
                continue;
            }
            out.push('"');
            j += 1;
            let closing: Vec<char> = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            while j < chars.len() && !chars[j..].starts_with(&closing) {
                out.push(blank(chars[j]));
                j += 1;
            }
            out.push('"');
            i = (j + closing.len()).min(chars.len());
        } else if c == '"' {
            out.push('"');
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < chars.len() {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
            out.push('"');
            i += 1;
        } else if c == '\'' && (next == Some('\\') || chars.get(i + 2) == Some(&'\'')) {
            // A char literal; a lone quote is a lifetime and stays as is.
            out.push('\'');
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < chars.len() {
                    out.push(' ');
                    i += 1;
                }
            }
            out.push('\'');
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> Vec<String> {
        scan_source("lib.rs", source)
    }

    #[test]
    fn pure_code_passes() {
        let source = "use std::collections::HashMap;\nuse std::{fmt, ops::Range};\nfn f<'a>(x: &'a str) -> char { 'x' }\n";
        assert_eq!(scan(source), Vec::<String>::new());
    }

    #[test]
    fn std_fs_fails_with_line() {
        let violations = scan("fn f() {}\nuse std::fs::read;\n");
        assert_eq!(
            violations,
            ["lib.rs:2: pure crates may not use std::fs (rule R2)"]
        );
    }

    #[test]
    fn grouped_import_fails() {
        let violations = scan("use std::{collections::HashMap, env};");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("std::env"));
    }

    #[test]
    fn absolute_path_and_nested_group_fail() {
        assert_eq!(scan("let _ = ::std::thread::spawn(f);").len(), 1);
        assert_eq!(scan("use std::{io::{self, Write}, process};").len(), 1);
    }

    #[test]
    fn printing_and_clocks_fail() {
        assert_eq!(scan("fn f() { println!(\"hi\"); }").len(), 1);
        assert_eq!(scan("fn f() { eprint!(\"hi\"); }").len(), 1);
        assert_eq!(scan("let t = Instant::now();").len(), 1);
        assert_eq!(scan("let out = io::stdout();").len(), 1);
    }

    #[test]
    fn comments_and_strings_are_ignored() {
        let source = "// use std::fs;\n/* std::net /* nested */ std::env */\nlet s = \"std::process\";\nlet r = r#\"println!\"#;\nlet c = '\"';\n";
        assert_eq!(scan(source), Vec::<String>::new());
    }

    #[test]
    fn longer_identifiers_are_not_matched() {
        assert_eq!(
            scan("let mystd = 1; let instant_count = 2; fn stdout_like() {}"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn unlisted_dependency_fails() {
        let closures = [(
            "syntax".to_string(),
            vec!["rowan".to_string(), "rayon".to_string()],
        )];
        let violations = check_dependencies(&closures);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("`rayon`"));
    }
}
