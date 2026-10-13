//! `cargo xtask oracle lsp-import`: elm-language-server's own provider
//! tests as language-server cases, named `els-*`.
//!
//! Its tests write each project as a template string, files split at
//! `--@ File.elm` lines, and point into it with comment markers. Each
//! test file's helper reads the markers its own way, and this reads them
//! the same way, into exact positions, so a case never depends on the
//! driver's markers. The fixture packages stand in for its stub modules,
//! so `elm make` and the server see a real project, and a test whose
//! project needs anything else (an interpolated string, a table of
//! cases) is left out and counted.
//!
//! Left out: find tests and file events, whose
//! helpers build workspaces of their own, and the type checker's
//! diagnostics, which nomi takes from the compiler (DECISIONS.md,
//! 2026-10-12).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use nomi_testkit::lsp::Position;
use serde_json::{Map, Value, json};

use super::lsp::SUITE;

/// The reference sources' test directory.
const TESTS: &str = "reference/elm-language-server/test";

/// What a test file's helper does with its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Definition,
    References,
    Hover,
    TypeDefinition,
    Completion,
    Rename,
    Folding,
    CodeAction,
    Lint,
}

const FILES: &[(&str, Kind)] = &[
    ("definitionProviderTests", Kind::Definition),
    ("referencesProviderTests", Kind::References),
    ("hoverProvider.test.ts", Kind::Hover),
    ("typeDefinitionProvider.test.ts", Kind::TypeDefinition),
    ("completionProvider.test.ts", Kind::Completion),
    ("renameProvider.test.ts", Kind::Rename),
    ("foldingProvider.test.ts", Kind::Folding),
    ("codeActionTests", Kind::CodeAction),
    ("diagnosticTests/elmLsDiagnostics.test.ts", Kind::Lint),
];

pub(crate) fn run(root: &Path) -> Result<()> {
    let tests = root.join(TESTS);
    let cases = root.join(SUITE).join("cases");
    for entry in fs::read_dir(&cases)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("els-") {
            fs::remove_dir_all(entry.path())?;
        }
    }
    let (mut written, mut skipped) = (0, 0);
    for (path, kind) in FILES {
        let path = tests.join(path);
        let files = if path.is_dir() {
            let mut files: Vec<_> = fs::read_dir(&path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<_, _>>()?;
            files.retain(|f| f.to_string_lossy().ends_with(".test.ts"));
            files.sort();
            files
        } else {
            vec![path]
        };
        for file in files {
            let text =
                fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let stem = file.file_name().unwrap().to_string_lossy();
            let stem = kebab(stem.trim_end_matches(".test.ts"));
            for (n, test) in tests_of(&text).into_iter().enumerate() {
                let Some(case) = convert(*kind, &test) else {
                    skipped += 1;
                    continue;
                };
                let dir = cases.join(format!("els-{stem}-{:03}", n + 1));
                write(&dir, &test.title, case)?;
                written += 1;
            }
        }
    }
    eprintln!("  {written} cases written, {skipped} tests left out");
    Ok(())
}

/// One test: its title, its project, and the rest of its body.
struct Test {
    title: String,
    source: Option<String>,
    body: String,
}

/// A converted case: its files, by path, and its session.
struct Case {
    files: BTreeMap<String, String>,
    session: Map<String, Value>,
}

fn write(dir: &Path, title: &str, case: Case) -> Result<()> {
    for (path, text) in &case.files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, text)?;
    }
    let mut session = Map::new();
    session.insert("title".into(), json!(title));
    session.extend(case.session);
    let text = serde_json::to_string_pretty(&Value::Object(session))? + "\n";
    fs::write(dir.join("session.json"), text)?;
    Ok(())
}

/// The tests of a test file, in order: each `it(` or `test(` with a
/// literal title, its body up to the next, and its project, if it has
/// one without interpolation.
fn tests_of(text: &str) -> Vec<Test> {
    let mut starts = Vec::new();
    for keyword in ["it(", "test("] {
        for (at, _) in text.match_indices(keyword) {
            let before = text[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '_') {
                continue;
            }
            let rest = text[at + keyword.len()..].trim_start();
            let Some(quote) = rest
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\'' | '`'))
            else {
                continue;
            };
            let Some(end) = rest[1..].find(quote) else {
                continue;
            };
            starts.push((at, rest[1..=end].to_owned()));
        }
    }
    starts.sort();
    let mut tests = Vec::new();
    for (i, (at, title)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(text.len(), |(next, _)| *next);
        let body = &text[*at..end];
        tests.push(Test {
            title: title.clone(),
            source: source_of(body),
            body: body.to_owned(),
        });
    }
    tests
}

/// The template string assigned to `name` in `body`, unescaped; none if
/// it interpolates.
fn template(body: &str, name: &str) -> Option<String> {
    let mut search = 0;
    while let Some(found) = body[search..].find(name) {
        let at = search + found;
        search = at + name.len();
        let before = body[..at].trim_end();
        if !(before.ends_with("const") || before.ends_with("let")) {
            continue;
        }
        let rest = body[search..].trim_start().strip_prefix('=')?.trim_start();
        return template_at(rest);
    }
    None
}

/// The helpers whose first argument is the project.
const HELPERS: &[&str] = &[
    "testHover",
    "testCodeAction",
    "getTypeDefinition",
    "testDefinition",
    "testReferences",
    "testCompletions",
    "testDiagnostics",
    "testRename",
    "testPrepareRename",
];

/// The project a test gives its helper: its `source`, or a template
/// string written as the helper's first argument.
fn source_of(body: &str) -> Option<String> {
    if let Some(source) = template(body, "source") {
        return Some(source);
    }
    let (at, helper) = (HELPERS.iter())
        .filter_map(|h| Some((body.find(&format!("{h}("))?, h)))
        .min()?;
    template_at(body[at + helper.len() + 1..].trim_start())
}

/// The template string `rest` starts with, unescaped; none if it
/// interpolates.
fn template_at(rest: &str) -> Option<String> {
    let rest = rest.strip_prefix('`')?;
    let mut text = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '`' => return Some(text),
            '\\' => match chars.next()? {
                'n' => text.push('\n'),
                't' => text.push('\t'),
                other => text.push(other),
            },
            '$' if chars.as_str().starts_with('{') => return None,
            c => text.push(c),
        }
    }
    None
}

/// `sourceParser.getSourceFiles`: the files of a project, split at
/// `--@ File.elm` lines, or all of it as `Main.elm`.
fn source_files(source: &str) -> Vec<(String, String)> {
    let header = |line: &str| {
        let at = line.find("--@ ")?;
        let name: String = line[at + 4..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == '/' || *c == '.')
            .collect();
        name.ends_with(".elm").then_some(name)
    };
    if !source.lines().any(|l| header(l).is_some()) {
        return vec![("Main.elm".to_owned(), source.to_owned())];
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for line in source.split('\n') {
        if let Some(name) = header(line) {
            files.retain(|(n, _)| *n != name);
            files.push((name, String::new()));
        } else if let Some((_, text)) = files.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    files
}

/// Where a file of a project lies in the case: `tests/` files as they
/// are, the rest under `src/`.
fn path_of(name: &str) -> String {
    if name.starts_with("tests/") {
        name.to_owned()
    } else {
        format!("src/{name}")
    }
}

/// The UTF-16 column of byte `at` in `line`.
fn column(line: &str, at: usize) -> u32 {
    line[..at].encode_utf16().count() as u32
}

/// The invoke position as `getInvokePositionFromSource` and
/// `getReferencesTestFromSource` read it: the first `--^` of the last line
/// that has one.
fn invoke(files: &[(String, String)]) -> Option<(String, Position)> {
    let mut found = None;
    for (name, text) in files {
        for (line, s) in text.split('\n').enumerate() {
            if let Some(at) = s.find("--^") {
                let position = Position {
                    line: (line as u32).checked_sub(1)?,
                    character: column(s, at) + 2,
                };
                found = Some((name.clone(), position));
            }
        }
    }
    found
}

/// `getCaretPositionFromSource`: the last `{-caret-}`, taken out.
fn caret(files: &mut [(String, String)]) -> Option<(String, Position)> {
    let mut found = None;
    for (name, text) in files.iter_mut() {
        let lines: Vec<String> = text
            .split('\n')
            .enumerate()
            .map(|(line, s)| {
                if let Some(at) = s.find("{-caret-}") {
                    let position = Position {
                        line: line as u32,
                        character: column(s, at),
                    };
                    found = Some((name.clone(), position));
                }
                s.replacen("{-caret-}", "", 1)
            })
            .collect();
        *text = lines.join("\n");
    }
    found
}

/// `getTargetPositionFromSource`: the first two `--^`, the second's end
/// one past its `^` and counted as if the first marker's line were gone.
fn target(files: &[(String, String)]) -> Option<(Position, Position)> {
    let mut start: Option<Position> = None;
    let mut end: Option<Position> = None;
    for (_, text) in files {
        for (line, s) in text.split('\n').enumerate() {
            let mut offset = 0;
            while let Some(found) = s[offset..].find("--^") {
                let at = offset + found;
                let character = column(s, at) + 2;
                let line_index = (line as u32).checked_sub(1)?;
                match start {
                    None => {
                        start = Some(Position {
                            line: line_index,
                            character,
                        });
                        offset = at + 2;
                    }
                    Some(first) if end.is_none() => {
                        let line = if first.line == line_index {
                            line_index
                        } else {
                            line_index - 1
                        };
                        end = Some(Position {
                            line,
                            character: character + 1,
                        });
                        break;
                    }
                    Some(_) => break,
                }
            }
        }
    }
    let start = start?;
    Some((start, end.unwrap_or(start)))
}

fn trim_trailing_whitespace(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').map(str::trim_end).collect();
    lines.join("\n")
}

fn strip_comment_lines(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').filter(|l| !l.contains("--")).collect();
    lines.join("\n")
}

/// The string literal after `call(source, ` in `body`.
fn string_argument(body: &str, call: &str) -> Option<String> {
    let at = body.find(&format!("{call}("))?;
    let rest = body[at..].split_once(',')?.1.trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let end = rest[1..].find(quote)?;
    Some(rest[1..=end].to_owned())
}

fn convert(kind: Kind, test: &Test) -> Option<Case> {
    // Some of its projects indent with tabs, which elm rejects, and
    // elm-language-server then shows nothing elm says (DECISIONS.md,
    // 2026-10-12). Its tests never compile them, so they become spaces,
    // before any marker is read.
    let source = &test.source.as_deref()?.replace('\t', "    ");
    let request = |method: &str, file: &str, at: Value, params: Value| {
        let mut step = json!({ "method": method, "file": path_of(file) });
        if !at.is_null() {
            step["at"] = at;
        }
        if params.as_object().is_some_and(|p| !p.is_empty()) {
            step["params"] = params;
        }
        step
    };
    let (files, open, requests, settings) = match kind {
        Kind::Definition | Kind::References | Kind::Hover => {
            let files = source_files(source);
            let (file, position) = invoke(&files)?;
            let (method, params) = match kind {
                Kind::Definition => ("textDocument/definition", json!({})),
                Kind::References => (
                    "textDocument/references",
                    json!({ "context": { "includeDeclaration": true } }),
                ),
                _ => ("textDocument/hover", json!({})),
            };
            let step = request(method, &file, json!(position), params);
            (files, vec![file], vec![step], json!({}))
        }
        Kind::TypeDefinition | Kind::Completion => {
            let mut files = source_files(source);
            let (file, position) = caret(&mut files)?;
            let (method, params) = if kind == Kind::Completion {
                let text = &files.iter().find(|(n, _)| *n == file)?.1;
                let line = text.split('\n').nth(position.line as usize)?;
                let before: Vec<u16> = line
                    .encode_utf16()
                    .take(position.character as usize)
                    .collect();
                let context = if before.last() == Some(&u16::from(b'.')) {
                    json!({ "triggerKind": 2, "triggerCharacter": "." })
                } else {
                    json!({ "triggerKind": 1 })
                };
                ("textDocument/completion", json!({ "context": context }))
            } else {
                ("textDocument/typeDefinition", json!({}))
            };
            let step = request(method, &file, json!(position), params);
            (files, vec![file], vec![step], json!({}))
        }
        Kind::Rename => {
            // A test prepares, renames, or both; each helper reads the
            // markers its own way, which only differs in trailing space.
            let prepare = test.body.contains("testPrepareRename(");
            let rename = string_argument(&test.body, "testRename");
            let text = if rename.is_some() {
                trim_trailing_whitespace(source)
            } else {
                source.to_owned()
            };
            let files = source_files(&text);
            let (start, _) = target(&files)?;
            let mut steps = Vec::new();
            if prepare {
                steps.push(request(
                    "textDocument/prepareRename",
                    "Test.elm",
                    json!(start),
                    json!({}),
                ));
            }
            if let Some(name) = rename {
                let params = json!({ "newName": name });
                steps.push(request(
                    "textDocument/rename",
                    "Test.elm",
                    json!(start),
                    params,
                ));
            }
            if steps.is_empty() {
                return None;
            }
            (files, vec!["Test.elm".to_owned()], steps, json!({}))
        }
        Kind::Folding => {
            let files = source_files(source);
            let file = files.first()?.0.clone();
            let step = request("textDocument/foldingRange", &file, Value::Null, json!({}));
            (files, vec![file], vec![step], json!({}))
        }
        Kind::CodeAction => {
            let mut files = source_files(&trim_trailing_whitespace(source));
            let (start, end) = target(&files)?;
            let test_file = files.iter_mut().find(|(n, _)| n == "Test.elm")?;
            test_file.1 = strip_comment_lines(&test_file.1);
            let at = json!({ "start": start, "end": end });
            let step = request("textDocument/codeAction", "Test.elm", at, json!({}));
            let settings = json!({ "disableElmLSDiagnostics": false });
            (files, vec!["Test.elm".to_owned()], vec![step], settings)
        }
        Kind::Lint => {
            let mut files = source_files(source);
            files.push((
                "../elm-analyse.json".to_owned(),
                "{ \"checks\": { \"SingleFieldRecord\": true } }\n".to_owned(),
            ));
            let open = (files.iter())
                .map(|(n, _)| n.clone())
                .filter(|n| n.ends_with(".elm"))
                .collect();
            let settings = json!({ "disableElmLSDiagnostics": false });
            (files, open, Vec::new(), settings)
        }
    };
    let mut session = Map::new();
    if settings.as_object().is_some_and(|s| !s.is_empty()) {
        session.insert("settings".into(), settings);
    }
    let open: Vec<String> = open.iter().map(|f| path_of(f)).collect();
    session.insert("open".into(), json!(open));
    if !requests.is_empty() {
        session.insert("requests".into(), json!(requests));
    }
    let files = files
        .into_iter()
        .map(|(name, text)| match name.strip_prefix("../") {
            Some(top) => (top.to_owned(), text),
            None => (path_of(&name), text),
        })
        .collect();
    Some(Case { files, session })
}

/// `functionDefinition` as `function-definition`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_read_as_its_helpers_read_them() {
        let files = source_files("\nfoo x y =  x + y\n    --X      --^\n");
        assert_eq!(
            invoke(&files),
            Some((
                "Main.elm".to_owned(),
                Position {
                    line: 1,
                    character: 15
                }
            ))
        );
        let files =
            source_files("\n--@ Test.elm\nmodule Test exposing (..)\n\nf a =\n    a\n  --^\n");
        assert_eq!(files[0].0, "Test.elm");
        let (start, end) = target(&files).unwrap();
        assert_eq!(
            (start, end),
            (
                Position {
                    line: 3,
                    character: 4
                },
                Position {
                    line: 3,
                    character: 4
                }
            )
        );
        let mut files = source_files("\nf = Maybe.{-caret-}\n");
        let (_, position) = caret(&mut files).unwrap();
        assert_eq!(
            position,
            Position {
                line: 1,
                character: 10
            }
        );
        assert_eq!(files[0].1, "\nf = Maybe.\n");
    }

    #[test]
    fn template_strings_unescape_and_refuse_interpolation() {
        let body = "it(\"x\", () => {\n const source = `a \\` b\\\\n`;\n";
        assert_eq!(template(body, "source").as_deref(), Some("a ` b\\n"));
        assert_eq!(template("const source = `${x}`", "source"), None);
    }
}
