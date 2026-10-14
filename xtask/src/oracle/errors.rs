//! `cargo xtask oracle names`, `types`, `patterns` and `docs`: what each
//! pinned elm reports on projects that get names, types, patterns or
//! documentation wrong, for the engine's error-report ratchets. Each
//! suite's curated cases, under `crates/engine/tests/<suite>/cases`, aim
//! at one of upstream's reports (`Reporting/Error/Canonicalize.hs`,
//! `Type.hs`, `Pattern.hs`, `Docs.hs`) or at what `docs.json` says. The
//! names and types suites also edit the seeds under `seeds/` the ways
//! people get names or types wrong (a literal of another type, two
//! arguments swapped, an annotation edited), and `mutations.tsv` lists
//! the edits, so the tests make the same projects. Every project builds
//! with the fixture packages.

use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, child_token, parse_module};
use nomi_testkit::edit::{self, Edit};
use nomi_testkit::transcript::{Case, cases};

use super::modules::record_projects;
use super::sandbox::corpus_releases;
use super::{FIXTURE_PACKAGES, generated};
use crate::references::References;

/// A suite of projects with errors.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Suite {
    Names,
    Types,
    Patterns,
    Docs,
}

impl Suite {
    /// The suite's name: its command and its directory's.
    fn name(self) -> &'static str {
        match self {
            Suite::Names => "names",
            Suite::Types => "types",
            Suite::Patterns => "patterns",
            Suite::Docs => "docs",
        }
    }

    /// The suite's edits of `seed`, whose project is `case`.
    fn mutations(self, seed: &str, case: &Case) -> Vec<Edit> {
        match self {
            Suite::Names => names(seed, case),
            Suite::Types => types(seed, case),
            Suite::Patterns | Suite::Docs => Vec::new(),
        }
    }
}

/// Records `suite`: its cases, and its seeds with their mutations where
/// it has seeds.
pub(crate) fn run(root: &Path, refs: &References, suite: Suite) -> Result<()> {
    let dir = root.join("crates/engine/tests").join(suite.name());
    let mut projects: Vec<(String, Case)> = (cases(&dir.join("cases")).into_iter())
        .map(|(stem, case)| (format!("case/{stem}"), case))
        .collect();
    let mut table = format!("{}\n{}\n", generated(suite.name()), edit::HEADER);
    if dir.join("seeds").is_dir() {
        for (seed, case) in cases(&dir.join("seeds")) {
            projects.push((format!("seed/{seed}"), case.clone()));
            for edit in suite.mutations(&seed, &case) {
                table.push_str(&edit.to_line());
                table.push('\n');
                projects.push((format!("mutation/{}", edit.id), edit.apply(&case)));
            }
        }
    }
    fs::write(dir.join("mutations.tsv"), table)?;
    let pinned = corpus_releases(root, &FIXTURE_PACKAGES)?;
    record_projects(refs, &dir, suite.name(), &pinned, &projects).context("recording")
}

/// The edits of one seed, each numbered after the edits of its kind
/// before it.
struct Edits<'a> {
    seed: &'a str,
    /// The file the edits being made are in.
    file: &'a str,
    edits: Vec<Edit>,
    made: HashMap<&'static str, usize>,
}

impl<'a> Edits<'a> {
    fn new(seed: &'a str) -> Edits<'a> {
        Edits {
            seed,
            file: "",
            edits: Vec::new(),
            made: HashMap::new(),
        }
    }

    fn push(&mut self, kind: &'static str, range: Range<usize>, text: String, note: String) {
        let n = self.made.entry(kind).or_default();
        self.edits.push(Edit {
            id: format!("{}/{kind}/{n}", self.seed),
            seed: self.seed.to_owned(),
            file: self.file.to_owned(),
            start: range.start,
            end: range.end,
            text,
            note,
        });
        *n += 1;
    }
}

/// The ways `case`'s modules get names wrong, in a stable order.
fn names(seed: &str, case: &Case) -> Vec<Edit> {
    let mut edits = Edits::new(seed);
    for (file, text) in case.files.iter().filter(|(f, _)| f.ends_with(".elm")) {
        edits.file = file;
        for node in parse_module(text).syntax().descendants() {
            match node.kind() {
                // A misspelled reference to a value, type or constructor.
                SyntaxKind::NameExpr | SyntaxKind::NamedType | SyntaxKind::ConstructorPattern => {
                    if let Some(name) = last_ident(&node) {
                        let note = format!("{} misspelled", name.text());
                        edits.push("rename", range(&name), typo(name.text()), note);
                    }
                }
                SyntaxKind::ExposingList => exposing(&node, &mut edits),
                // An import left out.
                SyntaxKind::Import => {
                    let Range { start, end } = trimmed(&node);
                    let note = format!("{} left out", node.text().to_string().trim());
                    let end = (end + 1).min(text.len());
                    edits.push("unimport", start..end, String::new(), note);
                }
                // A declaration written twice.
                SyntaxKind::ValueDecl | SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl
                    if node
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::Module) =>
                {
                    let declaration = trimmed(&node);
                    let end = declaration.end;
                    let copy = format!("\n\n\n{}", &text[declaration]);
                    let note = "a declaration written twice".to_owned();
                    edits.push("duplicate", end..end, copy, note);
                }
                SyntaxKind::Definition => definition(&node, &mut edits),
                _ => {}
            }
            if node.kind() == SyntaxKind::NamedType {
                arity(&node, &mut edits);
            }
        }
    }
    edits.edits
}

/// A name taken out of an import's exposing list, or a misspelled one in
/// the module's own.
fn exposing(node: &SyntaxNode, edits: &mut Edits<'_>) {
    let items: Vec<SyntaxNode> = node
        .children()
        .filter(|c| ast::Exposed::can_cast(c.kind()))
        .collect();
    let in_import = node
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::Import);
    for (i, item) in items.iter().enumerate() {
        if in_import {
            if items.len() < 2 {
                continue;
            }
            // With the comma after it, or the last one's before it.
            let Range { start, end } = trimmed(item);
            let range = match items.get(i + 1) {
                Some(next) => start..trimmed(next).start,
                None => trimmed(&items[i - 1]).end..end,
            };
            let note = format!("{} not imported", item.text());
            edits.push("unexpose", range, String::new(), note);
        } else if let Some(name) = item.first_token() {
            let note = format!("exposing {} misspelled", name.text());
            edits.push("export", range(&name), typo(name.text()), note);
        }
    }
}

/// A type given one argument fewer.
fn arity(node: &SyntaxNode, edits: &mut Edits<'_>) {
    let args: Vec<SyntaxNode> = node
        .children()
        .filter(|c| ast::Type::can_cast(c.kind()))
        .collect();
    if let Some(last) = args.last() {
        let start = match args.len() {
            1 => node.first_token().map_or(0, |t| range(&t).end),
            n => trimmed(&args[n - 2]).end,
        };
        let note = "a type argument left out".to_owned();
        edits.push("arity", start..trimmed(last).end, String::new(), note);
    }
}

/// The mutations of one definition: a parameter named as another, a
/// local named as a parameter, and a value defined as itself.
fn definition(node: &SyntaxNode, edits: &mut Edits<'_>) {
    let params: Vec<SyntaxToken> = node
        .children()
        .filter(|c| c.kind() == SyntaxKind::VarPattern)
        .filter_map(|c| c.first_token())
        .collect();
    if let [first, second, ..] = &params[..] {
        let note = format!("{} named {}", second.text(), first.text());
        edits.push(
            "duplicate-param",
            range(second),
            first.text().to_owned(),
            note,
        );
    }
    if let Some(first) = params.first() {
        // Locals bound inside the body, renamed to the first parameter.
        let body = node.children().find(|c| ast::Expr::can_cast(c.kind()));
        for local in body.iter().flat_map(|b| b.descendants()) {
            if local.kind() == SyntaxKind::VarPattern
                && let Some(name) = local.first_token()
                && name.text() != first.text()
            {
                let note = format!("{} named {}", name.text(), first.text());
                edits.push("shadow", range(&name), first.text().to_owned(), note);
            }
        }
    }
    let name = child_token(node, SyntaxKind::LowerIdent);
    let body = node
        .children()
        .filter(|c| ast::Expr::can_cast(c.kind()))
        .last();
    if params.is_empty()
        && node.children().all(|c| !ast::Pattern::can_cast(c.kind()))
        && let (Some(name), Some(body)) = (name, body)
        && body.kind() != SyntaxKind::LambdaExpr
    {
        let note = format!("{} defined as itself", name.text());
        edits.push("cycle", trimmed(&body), name.text().to_owned(), note);
    }
}

/// The ways `case`'s modules get types wrong, in a stable order.
fn types(seed: &str, case: &Case) -> Vec<Edit> {
    let mut edits = Edits::new(seed);
    for (file, text) in case.files.iter().filter(|(f, _)| f.ends_with(".elm")) {
        edits.file = file;
        for node in parse_module(text).syntax().descendants() {
            match node.kind() {
                // A literal of another type.
                SyntaxKind::LiteralExpr => {
                    let Some(token) = node.first_token() else {
                        continue;
                    };
                    let other = match token.kind() {
                        SyntaxKind::Int | SyntaxKind::Float => "\"x\"",
                        SyntaxKind::String | SyntaxKind::MultilineString => "0",
                        SyntaxKind::Char => "\"c\"",
                        _ => continue,
                    };
                    let note = format!("{} written as {other}", token.text());
                    edits.push("literal", range(&token), other.to_owned(), note);
                }
                // The first two arguments of a call swapped.
                SyntaxKind::CallExpr => {
                    let parts: Vec<SyntaxNode> = node.children().collect();
                    if let [_, first, second, ..] = &parts[..] {
                        let (a, b) = (trimmed(first), trimmed(second));
                        let swapped = format!(
                            "{}{}{}",
                            &text[b.clone()],
                            &text[a.end..b.start],
                            &text[a.clone()]
                        );
                        let note = "two arguments swapped".to_owned();
                        edits.push("swap", a.start..b.end, swapped, note);
                    }
                }
                // A type in an annotation changed.
                SyntaxKind::NamedType
                    if node.ancestors().any(|a| a.kind() == SyntaxKind::Signature) =>
                {
                    let Some(name) = node
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .filter(|t| t.kind() == SyntaxKind::UpperIdent)
                        .last()
                    else {
                        continue;
                    };
                    let other = if name.text() == "Int" {
                        "String"
                    } else {
                        "Int"
                    };
                    let note = format!("{} annotated as {other}", name.text());
                    edits.push("annotation", range(&name), other.to_owned(), note);
                }
                _ => {}
            }
        }
    }
    edits.edits
}

fn last_ident(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::LowerIdent | SyntaxKind::UpperIdent))
        .last()
}

fn range(token: &SyntaxToken) -> Range<usize> {
    token.text_range().into()
}

/// From `node`'s first token that isn't trivia to its last; to the node's
/// end where it has only one.
fn trimmed(node: &SyntaxNode) -> Range<usize> {
    let mut tokens = (node.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia());
    let first = tokens
        .next()
        .map_or_else(|| node.text_range().start(), |t| t.text_range().start());
    let last = tokens
        .last()
        .map_or_else(|| node.text_range().end(), |t| t.text_range().end());
    first.into()..last.into()
}

/// `name` with its last two letters swapped, or doubled when it has one.
fn typo(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    match chars.len() {
        0 | 1 => format!("{name}{name}"),
        n if chars[n - 1] == chars[n - 2] => format!("{name}x"),
        n => {
            let mut swapped = chars;
            swapped.swap(n - 1, n - 2);
            swapped.into_iter().collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_are_numbered_by_kind_across_files() {
        let module = "module A exposing (a)\n\n\na =\n    f 1 \"s\"\n";
        let file = |name: &str| (name.to_owned(), module.to_owned());
        let case = Case {
            files: vec![file("src/A.elm"), file("src/B.elm")],
            ..Case::default()
        };
        let ids: Vec<String> = types("seed", &case).into_iter().map(|e| e.id).collect();
        let expected = [
            "seed/swap/0",
            "seed/literal/0",
            "seed/literal/1",
            "seed/swap/1",
            "seed/literal/2",
            "seed/literal/3",
        ];
        assert_eq!(ids, expected);
    }
}
