//! The files a run takes, as elm-test resolves its arguments
//! (`FindTests.resolveGlobs`): `tests/` when there are none, a directory's
//! `.elm` files below it, a file as given, and an argument that names
//! nothing, a glob from the project's root.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use nomi_project::Outline;
use walkdir::{DirEntry, WalkDir};

/// Directories elm-test never looks in (`FindTests.ignoredDirsGlobs`).
pub(crate) const IGNORED: [&str; 2] = ["elm-stuff", "node_modules"];

/// What elm-test reads of a project (`Project.init`).
#[derive(Clone, Debug)]
pub struct Project {
    /// The directory of its `elm.json`.
    pub root: PathBuf,
    /// `tests/`, which need not exist.
    pub tests_dir: PathBuf,
    /// Where test files may be: the project's source directories, `src/`
    /// for a package, and `tests/` where it exists.
    pub source_dirs: Vec<PathBuf>,
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
        let written = outline.source_dirs();
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
    // The project's tree, walked once for every glob.
    let mut tree = None;
    let mut found: Vec<PathBuf> = Vec::new();
    let mut seen = HashSet::new();
    for path in given {
        let matched = match fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => elm_files(&path),
            Ok(_) => vec![path],
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let pattern = relative(&path, &project.root);
                let pattern = Glob::new(&pattern.to_string_lossy().replace('\\', "/"));
                glob(
                    tree.get_or_insert_with(|| Entry::tree(&project.root)),
                    &pattern,
                )
            }
            Err(_) => Vec::new(),
        };
        found.extend(matched.into_iter().filter(|file| seen.insert(file.clone())));
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
    let tests = &project.tests_dir;
    let what = match fs::metadata(tests) {
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

/// Everything below `dir`, in name order, past the ignored directories
/// and hidden files.
fn walk(dir: &Path) -> impl Iterator<Item = DirEntry> {
    let skipped = |entry: &DirEntry| {
        let name = entry.file_name();
        name.as_encoded_bytes().starts_with(b".")
            || (entry.file_type().is_dir() && IGNORED.iter().any(|i| name == *i))
    };
    let walk = WalkDir::new(dir).min_depth(1).sort_by_file_name();
    (walk.into_iter())
        .filter_entry(move |entry| !skipped(entry))
        .filter_map(Result::ok)
}

fn is_elm(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("elm"))
}

/// Every `.elm` file below `dir`, past the ignored directories, by path.
fn elm_files(dir: &Path) -> Vec<PathBuf> {
    (walk(dir))
        .filter(|entry| !entry.file_type().is_dir() && is_elm(entry.path()))
        .map(DirEntry::into_path)
        .collect()
}

/// A path below the project's root, as globs see it.
struct Entry {
    path: PathBuf,
    /// Its path from the root, a part for each component.
    parts: Vec<String>,
    dir: bool,
}

impl Entry {
    /// Everything below `root`, as [`walk`] finds it.
    fn tree(root: &Path) -> Vec<Entry> {
        walk(root)
            .map(|entry| {
                let below = entry.path().strip_prefix(root);
                Entry {
                    parts: parts(below.expect("a walk stays below its root")),
                    dir: entry.file_type().is_dir(),
                    path: entry.into_path(),
                }
            })
            .collect()
    }
}

/// The files `glob` matches in `tree`: a directory it matches gives its
/// `.elm` files, which follow it in the tree.
fn glob(tree: &[Entry], glob: &Glob) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for (at, entry) in tree.iter().enumerate() {
        if !glob.matches(&entry.parts) {
            continue;
        }
        if entry.dir {
            let below = tree[at + 1..]
                .iter()
                .take_while(|e| e.path.starts_with(&entry.path));
            found.extend((below.filter(|e| !e.dir && is_elm(&e.path))).map(|e| e.path.clone()));
        } else {
            found.push(entry.path.clone());
        }
    }
    found
}

/// A glob, its braces expanded into patterns and their segments parsed:
/// `**` stands for any number of segments, and within one, `*`, `?` and
/// `[…]` match regardless of ASCII case.
struct Glob(Vec<Vec<Segment>>);

enum Segment {
    /// `**`.
    Any,
    Name(Vec<Token>),
}

enum Token {
    /// `*`.
    Star,
    /// `?`.
    One,
    /// `[…]`, or with `negated`, `[!…]` or `[^…]`.
    Class {
        negated: bool,
        class: Vec<u8>,
    },
    Byte(u8),
}

impl Glob {
    fn new(text: &str) -> Glob {
        let patterns = braces(text).into_iter().map(|pattern| {
            (pattern.split('/').filter(|s| !s.is_empty()))
                .map(Segment::parse)
                .collect()
        });
        Glob(patterns.collect())
    }

    /// Whether a path, by its parts, matches.
    fn matches(&self, parts: &[String]) -> bool {
        let any = |segment: &Segment| matches!(segment, Segment::Any);
        let star = |token: &Token| matches!(token, Token::Star);
        let one = |segment: &Segment, part: &String| match segment {
            Segment::Any => true,
            Segment::Name(tokens) => {
                wildcard(tokens, part.as_bytes(), star, |token, &b| token.matches(b))
            }
        };
        (self.0.iter()).any(|segments| wildcard(segments, parts, any, one))
    }
}

impl Segment {
    fn parse(text: &str) -> Segment {
        if text == "**" {
            return Segment::Any;
        }
        let mut tokens = Vec::new();
        let mut rest = text.as_bytes();
        while let Some((&first, after)) = rest.split_first() {
            rest = after;
            let class = (first == b'[')
                .then(|| after.iter().position(|&b| b == b']'))
                .flatten();
            tokens.push(match (first, class) {
                (b'[', Some(end)) => {
                    rest = &after[end + 1..];
                    let (negated, class) = match after[..end].split_first() {
                        Some((b'!' | b'^', class)) => (true, class),
                        _ => (false, &after[..end]),
                    };
                    Token::Class {
                        negated,
                        class: class.to_vec(),
                    }
                }
                (b'*', _) => Token::Star,
                (b'?', _) => Token::One,
                // An unclosed `[` is itself.
                (byte, _) => Token::Byte(byte),
            });
        }
        Segment::Name(tokens)
    }
}

impl Token {
    /// Whether it matches `byte`, as one that stands for one byte.
    fn matches(&self, byte: u8) -> bool {
        match self {
            Token::Star | Token::One => true,
            Token::Class { negated, class } => in_class(class, byte) != *negated,
            Token::Byte(b) => b.eq_ignore_ascii_case(&byte),
        }
    }
}

/// Whether `byte` is among a class's bytes and `a-z` ranges, regardless
/// of ASCII case.
fn in_class(class: &[u8], byte: u8) -> bool {
    let byte = byte.to_ascii_lowercase();
    let mut rest = class;
    while let [lo, after @ ..] = rest {
        let (hi, next) = match after {
            [b'-', hi, next @ ..] => (hi, next),
            _ => (lo, after),
        };
        if (lo.to_ascii_lowercase()..=hi.to_ascii_lowercase()).contains(&byte) {
            return true;
        }
        rest = next;
    }
    false
}

/// Whether `subject` matches `pattern`, whose stars stand for any number
/// of elements and whose other elements each match one. Greedy, going
/// back only to the last star, it never tries a state twice.
fn wildcard<P, S>(
    pattern: &[P],
    subject: &[S],
    star: impl Fn(&P) -> bool,
    one: impl Fn(&P, &S) -> bool,
) -> bool {
    let (mut p, mut s) = (0, 0);
    // After the last star: where the pattern goes on, and the subject's
    // element the star would take next.
    let mut back = None;
    while p < pattern.len() || s < subject.len() {
        if p < pattern.len() && star(&pattern[p]) {
            p += 1;
            back = Some((p, s));
        } else if p < pattern.len() && s < subject.len() && one(&pattern[p], &subject[s]) {
            p += 1;
            s += 1;
        } else if let Some((after, taken)) = back
            && taken < subject.len()
        {
            back = Some((after, taken + 1));
            (p, s) = (after, taken + 1);
        } else {
            return false;
        }
    }
    true
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

/// `path` from `cwd`, made absolute and normalized as Node's
/// `path.resolve` does, links and all left as they are.
fn absolute(path: &Path, cwd: &Path) -> PathBuf {
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

/// `path` relative to `root`, as Node's `path.relative`: through `..`
/// where it isn't below it.
pub(crate) fn relative(path: &Path, root: &Path) -> PathBuf {
    let (path, root): (Vec<_>, Vec<_>) = (path.components().collect(), root.components().collect());
    let shared = path.iter().zip(&root).take_while(|(a, b)| a == b).count();
    let up = root[shared..].iter().map(|_| Component::ParentDir);
    up.chain(path[shared..].iter().copied()).collect()
}

/// `path`'s components, as text.
pub(crate) fn parts(path: &Path) -> Vec<String> {
    (path.components())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        Glob::new(pattern).matches(&parts(Path::new(path)))
    }

    #[test]
    fn globs_match_as_elm_test_reads_them() {
        assert!(matches("tests/**/*Test.elm", "tests/A/B/FooTest.elm"));
        assert!(matches("tests/**/*Test.elm", "tests/FooTest.elm"));
        assert!(matches("TESTS/*.ELM", "tests/One.elm"));
        assert!(matches("tests/[A-C]*.elm", "tests/Beta.elm"));
        assert!(!matches("tests/[!A-C]*.elm", "tests/Beta.elm"));
        assert!(matches("tests/?ne.elm", "tests/One.elm"));
        assert!(!matches("tests/*.elm", "tests/A/One.elm"));
        assert!(matches("tests/[A.elm", "tests/[A.elm"));
        assert_eq!(braces("a/{b,c}/d"), ["a/b/d", "a/c/d"]);
        // Each star tried at most once per position.
        let long = format!("{}.elm", "a".repeat(64));
        assert!(!matches(&"*a".repeat(24), &long));
        assert!(!matches(
            &format!("{}b", "**/".repeat(24)),
            &"a/".repeat(64)
        ));
    }

    #[test]
    fn paths_resolve_as_node_resolves_them() {
        assert_eq!(
            absolute(Path::new("../x/./y"), Path::new("/a/b")),
            PathBuf::from("/a/x/y")
        );
        assert_eq!(
            relative(Path::new("/a/b/c"), Path::new("/a")),
            Path::new("b/c")
        );
        assert_eq!(
            relative(Path::new("/x/c"), Path::new("/a/b")),
            Path::new("../../x/c")
        );
    }
}
