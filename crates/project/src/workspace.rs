//! A workspace's member patterns: paths relative to the root `nomi.toml`,
//! where `*` stands for any one directory. Matching is pure; finding the
//! directories a pattern names is the engine's.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path};

/// A member pattern from `[workspace] members`, kept as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    text: String,
    segments: Box<[Segment]>,
}

/// One component of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Name(String),
    /// `*`: any one component not starting with `.`.
    Any,
}

impl Pattern {
    /// The pattern `text`, unless it is empty, absolute, has `..` or uses
    /// `*` in part of a component.
    pub fn parse(text: &str) -> Option<Pattern> {
        let mut segments = Vec::new();
        for component in Path::new(text).components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => {
                    let name = name.to_str()?;
                    segments.push(match name {
                        "*" => Segment::Any,
                        _ if name.contains('*') => return None,
                        _ => Segment::Name(name.to_owned()),
                    });
                }
                Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
            }
        }
        (!segments.is_empty()).then(|| Pattern {
            text: text.to_owned(),
            segments: segments.into(),
        })
    }

    /// Whether `path`, relative to the root, is a directory this pattern
    /// names.
    pub fn matches(&self, path: &Path) -> bool {
        let mut names = path.components().filter(|c| *c != Component::CurDir);
        self.segments.iter().all(|segment| match names.next() {
            Some(Component::Normal(name)) => segment.matches(name),
            _ => false,
        }) && names.next().is_none()
    }
}

impl Segment {
    fn matches(&self, name: &OsStr) -> bool {
        match self {
            Segment::Name(literal) => name == literal.as_str(),
            Segment::Any => !name.as_encoded_bytes().starts_with(b"."),
        }
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_patterns_are_refused() {
        for text in [
            "",
            ".",
            "./",
            "/abs",
            "../x",
            "apps/../x",
            "a*b",
            "apps/web*",
            "**",
        ] {
            assert_eq!(Pattern::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_pattern_matches_whole_components() {
        let cases: &[(&str, &str, bool)] = &[
            ("apps/web", "apps/web", true),
            ("apps/web", "./apps/web", true),
            ("./apps/web/", "apps/web", true),
            ("apps/web", "apps/api", false),
            ("apps/web", "apps", false),
            ("apps/web", "apps/web/src", false),
            ("packages/*", "packages/shared", true),
            ("packages/*", "packages/.hidden", false),
            ("packages/*", "packages", false),
            ("packages/*", "packages/a/b", false),
            ("*/app", "web/app", true),
            ("*/app", "web/lib", false),
            ("apps/web", "../apps/web", false),
        ];
        for &(pattern, path, expected) in cases {
            let found = Pattern::parse(pattern)
                .expect(pattern)
                .matches(Path::new(path));
            assert_eq!(found, expected, "{pattern} against {path}");
        }
    }

    #[test]
    fn a_pattern_shows_as_written() {
        let pattern = Pattern::parse("./packages/*/").expect("a good pattern");
        assert_eq!(pattern.to_string(), "./packages/*/");
    }
}
