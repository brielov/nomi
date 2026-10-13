//! Which modules the test files are, and which of their values are tests
//! (`FindTests.findTests`, plan 10.2).
//!
//! elm-test takes a file's exposed values by syntax, then asks the
//! compiled JavaScript which are tests. nomi asks the types: a module's
//! exposed values whose type is elm-explorations/test's `Test`, in the
//! order elm-test would have found them.

use std::fs;
use std::path::{Path, PathBuf};

use nomi_compiler::canonical::Type;
use nomi_compiler::interface::Interface;
use nomi_syntax::ast::AstNode;
use nomi_syntax::{SyntaxKind, SyntaxNode, parse_module};

use crate::files::Project;

/// A test file: its module, and the values it may expose as tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestModule {
    pub name: String,
    pub path: PathBuf,
    /// The lowercase names it exposes, as elm-test finds them: its list's,
    /// or with `exposing (..)` each `name =` at the top, in order; none
    /// where it doesn't parse, which elm reports when it's built.
    pub candidates: Vec<String>,
}

/// The modules of `files`, each named by its path under the one source
/// directory of `project` holding it; elm-test's message where a file is
/// in none, in several, or makes no module name.
pub fn modules(project: &Project, files: &[PathBuf]) -> Result<Vec<TestModule>, String> {
    files
        .iter()
        .map(|path| {
            let dirs: Vec<&PathBuf> = (project.source_dirs.iter())
                .filter(|dir| path.starts_with(dir) && path != *dir)
                .collect();
            let dir = match dirs.as_slice() {
                [] => return Err(no_source_directory(path, project.package)),
                [dir] => *dir,
                several => return Err(several_source_directories(path, several, project)),
            };
            let parts: Vec<String> = (path.strip_prefix(dir).unwrap_or(path).components())
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let mut parts = parts;
            if let Some(last) = parts.last_mut()
                && let Some(stem) = last.strip_suffix(".elm")
            {
                *last = stem.to_owned();
            }
            let name = parts.join(".");
            if !parts.iter().all(|p| upper_name(p)) {
                return Err(bad_module_name(path, dir, &name));
            }
            let text = fs::read_to_string(path).unwrap_or_default();
            Ok(TestModule {
                name,
                path: path.clone(),
                candidates: candidates(&text),
            })
        })
        .collect()
}

/// The candidates of `module` whose type in its interface is `Test`, in
/// order.
pub fn tests(module: &TestModule, interface: &Interface) -> Vec<String> {
    (module.candidates.iter())
        .filter(|name| {
            interface
                .values
                .get(name.as_str())
                .is_some_and(|annotation| is_test(&annotation.ty))
        })
        .cloned()
        .collect()
}

/// Whether `ty` is elm-explorations/test's `Test`: the alias `Test.Test`,
/// or the type it stands for.
fn is_test(ty: &Type) -> bool {
    match ty {
        Type::Alias(home, name, args, _) => {
            &*home.package == "elm-explorations/test"
                && &*home.module == "Test"
                && name == "Test"
                && args.is_empty()
        }
        Type::Type(home, name, args) => {
            &*home.package == "elm-explorations/test"
                && &*home.module == "Test.Internal"
                && name == "Test"
                && args.is_empty()
        }
        _ => false,
    }
}

/// The lowercase names `text` exposes as elm-test's parser finds them.
fn candidates(text: &str) -> Vec<String> {
    let parse = parse_module(text);
    if !parse.errors().is_empty() {
        return Vec::new();
    }
    let root = parse.syntax();
    let child = |node: &SyntaxNode, kind| node.children().find(|c| c.kind() == kind);
    let Some(list) = child(&root, SyntaxKind::ModuleHeader)
        .and_then(|header| child(&header, SyntaxKind::ExposingList))
    else {
        return Vec::new();
    };
    // `exposing (..)`, not a type's `(..)`.
    let all = list
        .children_with_tokens()
        .any(|e| e.kind() == SyntaxKind::DotDot);
    if !all {
        return (list.children())
            .filter(|c| c.kind() == SyntaxKind::ExposedValue)
            .filter_map(|c| first_lower(&c))
            .collect();
    }
    // Each `name =` at the top: a value, which a test is.
    (root.children())
        .filter(|decl| decl.kind() == SyntaxKind::ValueDecl)
        .filter_map(|decl| child(&decl, SyntaxKind::Definition))
        .filter(|definition| {
            !definition
                .children()
                .any(|c| nomi_syntax::ast::Pattern::can_cast(c.kind()))
        })
        .filter_map(|definition| first_lower(&definition))
        .collect()
}

fn first_lower(node: &SyntaxNode) -> Option<String> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::LowerIdent)
        .map(|t| t.text().to_owned())
}

/// `Parser.isUpperName`: an uppercase letter, then letters, digits and `_`.
fn upper_name(part: &str) -> bool {
    let mut chars = part.chars();
    chars.next().is_some_and(char::is_uppercase) && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// `FindTests.missingSourceDirectoryError`.
fn no_source_directory(path: &Path, package: bool) -> String {
    let advice = if package {
        "Move it to tests/ or src/ in your project root."
    } else {
        "Move it to tests/ in your project root, or make sure it is covered by \"source-directories\" in your elm.json."
    };
    format!(
        "This file:\n\n{}\n\n…matches no source directory! Imports won't work then.\n\n{advice}",
        path.display()
    )
}

/// `FindTests.multipleSourceDirectoriesError`.
fn several_source_directories(path: &Path, dirs: &[&PathBuf], project: &Project) -> String {
    let listed: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    let note = if dirs.contains(&&project.tests_dir) {
        "Note: The tests/ folder counts as a source directory too (even if it isn't listed in your elm.json)!"
    } else {
        ""
    };
    format!(
        "This file:\n\n{}\n\n…matches more than one source directory:\n\n{}\n\nEdit \"source-directories\" in your elm.json and try to make it so no source directory contains another source directory!\n\n{note}",
        path.display(),
        listed.join("\n")
    )
    .trim_end()
    .to_owned()
}

/// `FindTests.badModuleNameError`.
fn bad_module_name(path: &Path, dir: &Path, name: &str) -> String {
    format!(
        "This file:\n\n{}\n\n…located in this directory:\n\n{}\n\n…is problematic. Trying to construct a module name from the parts after the directory gives:\n\n{name}\n\n…but module names need to look like for example:\n\nMain\nHttp.Helpers\n\nMake sure that all parts start with an uppercase letter and don't contain any spaces or anything like that.",
        path.display(),
        dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_found_as_elm_test_finds_them() {
        let listed = "module A exposing (b, C, d, E(..))\n\nb = 1\n";
        assert_eq!(candidates(listed), ["b", "d"]);
        let all =
            "module A exposing (..)\n\nx : Int\nx =\n    1\n\n\nf y =\n    y\n\n\nz =\n    2\n";
        assert_eq!(candidates(all), ["x", "z"]);
        assert!(candidates("module A exposing (..)\n\nx = (\n").is_empty());
    }
}
