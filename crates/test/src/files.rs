//! The files a run takes, as elm-test resolves its arguments
//! (`FindTests.resolveGlobs`): `tests/` when there are none, a directory's
//! `.elm` files below it, a file as given, and an argument that names
//! nothing, a glob from the project's root.

use std::fs;
use std::path::{Component, Path, PathBuf};

use nomi_project::Outline;

/// Directories elm-test never looks in.
const IGNORED: [&str; 2] = ["elm-stuff", "node_modules"];

/// What elm-test reads of a project (`Project.init`).
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    /// `tests/`, which need not exist.
    pub tests_dir: PathBuf,
    /// Where test files may be: the project's source directories, `src/`
    /// for a package, and `tests/` where it exists.
    pub source_dirs: Vec<PathBuf>,
    pub package: bool,
    pub outline: Outline,
    /// Its `elm.json`, which the generated project's dependencies are
    /// kept by.
    pub elm_json: Vec<u8>,
}

impl Project {
    /// The project whose `elm.json` is in `root` (`Project.init`).
    pub fn read(root: &Path) -> Result<Project, String> {
        let root = absolute(root, Path::new("/"));
        let bytes = fs::read(root.join("elm.json")).map_err(|e| e.to_string())?;
        // Decoded only: elm-test builds a project of its own, so this one
        // needn't be one elm builds (it may lack `elm/json`).
        let outline = (nomi_base::Compat::ALL.into_iter().rev())
            .find_map(|compat| Outline::decode(&bytes, compat).ok())
            .ok_or_else(|| "elm.json is not a project nomi reads".to_owned())?;
        let (package, written) = match &outline {
            Outline::App(app) => (
                false,
                (app.source_dirs.iter())
                    .filter_map(|d| d.path())
                    .collect::<Vec<_>>(),
            ),
            Outline::Pkg(_) => (true, vec!["src".to_owned()]),
        };
        let tests_dir = root.join("tests");
        let mut source_dirs: Vec<PathBuf> = written
            .iter()
            .map(|d| absolute(Path::new(d), &root))
            .collect();
        if !source_dirs.contains(&tests_dir) && tests_dir.exists() {
            source_dirs.push(tests_dir.clone());
        }
        Ok(Project {
            root,
            tests_dir,
            source_dirs,
            package,
            outline,
            elm_json: bytes,
        })
    }

    /// `validateTestsSourceDirs`: elm-test's message where a source
    /// directory is missing.
    pub fn validate(&self) -> Result<(), String> {
        for dir in &self.source_dirs {
            let problem = match fs::metadata(dir) {
                Ok(meta) if meta.is_dir() => continue,
                Ok(_) => "It exists but isn't a directory!".to_owned(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    "It doesn't exist though. Is it missing? Is there a typo?".to_owned()
                }
                Err(e) => format!("Failed to read that directory: {e}"),
            };
            return Err(format!(
                "The \"source-directories\" field in your elm.json lists the following directory:\n\n{}\n\n{problem}",
                dir.display()
            ));
        }
        Ok(())
    }
}

/// The test files `globs`, given from `cwd`, name in `project`, each once
/// and in order; elm-test's message where there are none.
pub fn resolve(project: &Project, cwd: &Path, globs: &[String]) -> Result<Vec<PathBuf>, String> {
    let given: Vec<PathBuf> = if globs.is_empty() {
        vec![project.tests_dir.clone()]
    } else {
        globs.iter().map(|g| absolute(Path::new(g), cwd)).collect()
    };
    let mut found: Vec<PathBuf> = Vec::new();
    for path in given {
        let matched = match fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => elm_files(&path),
            Ok(_) => vec![path],
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => glob(project, &path),
            Err(_) => Vec::new(),
        };
        for file in matched {
            if !found.contains(&file) {
                found.push(file);
            }
        }
    }
    if found.is_empty() {
        return Err(no_files(project, globs));
    }
    Ok(found)
}

/// `FindTests.noFilesFoundError`.
fn no_files(project: &Project, globs: &[String]) -> String {
    if !globs.is_empty() {
        return format!(
            "No files found matching:\n\n{}\n\nAre the above patterns correct? Maybe try running elm-test with no arguments?",
            globs.join("\n")
        );
    }
    let tests = project.root.join("tests");
    let what = match fs::metadata(&tests) {
        Ok(meta) if meta.is_dir() => "No .elm files found in the tests/ directory.".to_owned(),
        Ok(_) => format!(
            "Expected a directory but found something else at: {}\nCheck it out! Could you remove it?",
            tests.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "The tests/ directory does not exist.".to_owned()
        }
        Err(e) => format!("Failed to read the tests/ directory: {e}"),
    };
    format!(
        "{what}\n\nTo generate some initial tests to get things going: elm-test init\n\nAlternatively, if your project has tests in a different directory,\ntry calling elm-test with a glob such as: elm-test \"src/**/*Tests.elm\""
    )
}

/// Every `.elm` file below `dir`, past the ignored directories, by path.
fn elm_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, &mut |path, is_dir| {
        if !is_dir
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("elm"))
        {
            found.push(path.to_owned());
        }
    });
    found
}

/// Calls `visit` with everything below `dir`, by path, sorted, past the
/// ignored directories and hidden files.
fn walk(dir: &Path, visit: &mut dyn FnMut(&Path, bool)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(PathBuf, bool)> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| (e.path(), e.file_type().is_ok_and(|t| t.is_dir())))
        .collect();
    entries.sort();
    for (path, is_dir) in entries {
        if is_dir
            && IGNORED
                .iter()
                .any(|i| path.file_name().is_some_and(|n| n == *i))
        {
            continue;
        }
        visit(&path, is_dir);
        if is_dir {
            walk(&path, visit);
        }
    }
}

/// The files `pattern`, an absolute path, matches as a glob relative to
/// the project's root, regardless of case; a directory it matches gives
/// its `.elm` files.
fn glob(project: &Project, pattern: &Path) -> Vec<PathBuf> {
    let relative = relative(pattern, &project.root);
    let patterns = braces(&relative.replace('\\', "/"));
    let mut found = Vec::new();
    walk(&project.root, &mut |path, is_dir| {
        let rel = relative_path(path, &project.root);
        if patterns.iter().any(|p| matches(p, &rel)) {
            if is_dir {
                found.extend(elm_files(path));
            } else {
                found.push(path.to_owned());
            }
        }
    });
    found
}

/// `pattern` with each `{a,b}` expanded, as several patterns.
fn braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let mut depth = 0;
    let mut close = None;
    for (i, c) in pattern[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![pattern.to_owned()];
    };
    let (head, inner, tail) = (
        &pattern[..open],
        &pattern[open + 1..close],
        &pattern[close + 1..],
    );
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
        .iter()
        .flat_map(|part| braces(&format!("{head}{part}{tail}")))
        .collect()
}

/// Whether `path`, relative and `/`-separated, matches `pattern`, segment
/// by segment, `**` standing for any number of them.
fn matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segments(&pattern, &path)
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
        Some((first, rest)) => path.split_first().is_some_and(|(name, others)| {
            segment(first.as_bytes(), name.as_bytes()) && segments(rest, others)
        }),
    }
}

/// Whether the name matches the segment's `*`, `?` and `[…]`, regardless
/// of ASCII case.
fn segment(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|skip| segment(rest, &name[skip..])),
        Some((b'?', rest)) => !name.is_empty() && segment(rest, &name[1..]),
        Some((b'[', rest)) => {
            let Some(end) = rest.iter().position(|&b| b == b']') else {
                return name.first() == Some(&b'[') && segment(rest, &name[1..]);
            };
            let (class, after) = (&rest[..end], &rest[end + 1..]);
            let Some(&c) = name.first() else {
                return false;
            };
            let (negated, class) = match class.first() {
                Some(b'!' | b'^') => (true, &class[1..]),
                _ => (false, class),
            };
            let mut hit = false;
            let mut i = 0;
            while i < class.len() {
                if i + 2 < class.len() && class[i + 1] == b'-' {
                    let (lo, hi) = (
                        class[i].to_ascii_lowercase(),
                        class[i + 2].to_ascii_lowercase(),
                    );
                    hit |= (lo..=hi).contains(&c.to_ascii_lowercase());
                    i += 3;
                } else {
                    hit |= class[i].eq_ignore_ascii_case(&c);
                    i += 1;
                }
            }
            hit != negated && segment(after, &name[1..])
        }
        Some((&p, rest)) => {
            name.first().is_some_and(|c| c.eq_ignore_ascii_case(&p)) && segment(rest, &name[1..])
        }
    }
}

/// `path` from `cwd`, made absolute and normalized as Node's
/// `path.resolve` does, links and all left as they are.
pub fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `path` relative to `root`, `/`-separated, as Node's `path.relative`.
fn relative(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rest) => relative_path(&root.join(rest), root),
        Err(_) => {
            let (path, root): (Vec<_>, Vec<_>) =
                (path.components().collect(), root.components().collect());
            let shared = path.iter().zip(&root).take_while(|(a, b)| a == b).count();
            let mut parts: Vec<String> = vec!["..".to_owned(); root.len() - shared];
            parts.extend(
                path[shared..]
                    .iter()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned()),
            );
            parts.join("/")
        }
    }
}

fn relative_path(path: &Path, root: &Path) -> String {
    let rest = path.strip_prefix(root).unwrap_or(path);
    rest.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_as_elm_test_reads_them() {
        assert!(matches("tests/**/*Test.elm", "tests/A/B/FooTest.elm"));
        assert!(matches("tests/**/*Test.elm", "tests/FooTest.elm"));
        assert!(matches("TESTS/*.ELM", "tests/One.elm"));
        assert!(matches("tests/[A-C]*.elm", "tests/Beta.elm"));
        assert!(!matches("tests/[!A-C]*.elm", "tests/Beta.elm"));
        assert!(matches("tests/?ne.elm", "tests/One.elm"));
        assert!(!matches("tests/*.elm", "tests/A/One.elm"));
        assert_eq!(braces("a/{b,c}/d"), ["a/b/d", "a/c/d"]);
    }

    #[test]
    fn paths_resolve_as_node_resolves_them() {
        assert_eq!(
            absolute(Path::new("../x/./y"), Path::new("/a/b")),
            PathBuf::from("/a/x/y")
        );
        assert_eq!(relative(Path::new("/a/b/c"), Path::new("/a")), "b/c");
        assert_eq!(relative(Path::new("/x/c"), Path::new("/a/b")), "../../x/c");
    }
}
