//! The test suites of a project's `tests/` modules, as elm-language-server
//! finds them for test explorers (`findTestsProvider.ts`): each top-level
//! value whose expression is a call that makes a `Test`, a suite where it
//! is `describe` with a list of tests, each labelled by its one string.

use std::path::{Path, PathBuf};

use nomi_base::TextSize;
use nomi_compiler::types::{self, error};
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::{SyntaxKind, SyntaxNode};

use crate::symbols::{child, code_range, words};

/// A test or a suite of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestSuite {
    pub label: String,
    pub path: PathBuf,
    pub at: TextSize,
    /// A suite's tests; none for a test.
    pub tests: Option<Vec<TestSuite>>,
}

/// The suites of the test modules of the project at `root`: one per
/// module that has any, named after it.
pub fn find_tests(index: &Index, root: &Path) -> Vec<TestSuite> {
    let tests_dir = root.join("tests");
    let tests_dir = nomi_engine::canonical(&tests_dir).unwrap_or(tests_dir);
    let mut found = Vec::new();
    for module in &index.modules {
        if !module.path.starts_with(&tests_dir) {
            continue;
        }
        let finder = Finder::new(index, module);
        let top: Vec<TestSuite> = (module.root().children())
            .filter(|n| n.kind() == SyntaxKind::ValueDecl)
            .filter_map(|decl| child(&decl, SyntaxKind::Definition))
            .filter_map(|definition| finder.suite(finder.test_call(&definition)?))
            .collect();
        if !top.is_empty() {
            found.push(TestSuite {
                label: module.name.clone(),
                path: module.path.clone(),
                at: TextSize::new(0),
                tests: Some(top),
            });
        }
    }
    found
}

struct Finder<'a> {
    module: &'a Indexed,
    types: types::ExprTypes,
}

impl<'a> Finder<'a> {
    fn new(index: &Index, module: &'a Indexed) -> Finder<'a> {
        let types = (module.canonical.as_ref())
            .map(|c| types::infer_types(index.compat, c).1)
            .unwrap_or_default();
        Finder { module, types }
    }

    fn type_of(&self, node: &SyntaxNode) -> Option<&error::Type> {
        crate::hover::type_of(&self.module.root(), &self.types, code_range(node)?)
    }

    /// `findTestFunctionCall`: through `let`s, the first call in `node`,
    /// if it makes a `Test` or a function to one.
    fn test_call(&self, node: &SyntaxNode) -> Option<SyntaxNode> {
        let body = match node.kind() {
            SyntaxKind::Definition => node
                .children()
                .filter(|c| !crate::lint::is_pattern(c.kind()))
                .last(),
            _ => Some(node.clone()),
        }?;
        if body.kind() == SyntaxKind::LetExpr {
            return self.test_call(&body.children().last()?);
        }
        let call = body
            .descendants()
            .find(|n| n.kind() == SyntaxKind::CallExpr)?;
        let ty = self.type_of(&call)?;
        let returned = match ty {
            error::Type::Lambda(_, b, rest) => rest.last().unwrap_or(b),
            ty => ty,
        };
        (is_test(ty) || is_test(returned)).then_some(call)
    }

    /// `findTestSuite`: a call labelled by one string literal, a suite
    /// where it's `describe` with tests.
    fn suite(&self, call: SyntaxNode) -> Option<TestSuite> {
        let mut parts = call.children();
        let target = parts.next()?;
        let args: Vec<SyntaxNode> = parts.collect();
        let string = args
            .iter()
            .find(|a| self.type_of(a).is_some_and(is_string))?;
        let literals: Vec<String> = string
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::LiteralExpr)
            .filter_map(|n| {
                (n.children_with_tokens())
                    .filter_map(|e| e.into_token())
                    .find(|t| !t.kind().is_trivia())
            })
            .filter(|t| matches!(t.kind(), SyntaxKind::String | SyntaxKind::MultilineString))
            .map(|t| label_of(t.text()))
            .collect();
        let [label] = literals.as_slice() else {
            return None;
        };
        let at = code_range(&call)?.start();
        let leaf = TestSuite {
            label: label.clone(),
            path: self.module.path.clone(),
            at,
            tests: None,
        };
        if !self.is_describe(&target) {
            return Some(leaf);
        }
        let list = args
            .get(1)
            .and_then(|a| a.descendants().find(|n| n.kind() == SyntaxKind::ListExpr));
        let tests: Vec<TestSuite> = (list.iter())
            .flat_map(|l| l.children())
            .filter_map(|e| self.test_call(&e))
            .filter_map(|call| self.suite(call))
            .collect();
        (!tests.is_empty()).then_some(TestSuite {
            tests: Some(tests),
            ..leaf
        })
    }

    /// `isTestSuite`: whether `target` names `Test.describe` as the module
    /// writes it.
    fn is_describe(&self, target: &SyntaxNode) -> bool {
        let Some(name) = target
            .descendants()
            .find(|n| n.kind() == SyntaxKind::NameExpr)
            .map(|n| words(&n))
        else {
            return false;
        };
        let prefix = name.rsplit_once('.').map(|(p, _)| p.to_owned());
        let qualifier = match &prefix {
            Some(_) => self.qualifier("Test", "describe"),
            None => String::new(),
        };
        name == format!("{qualifier}describe")
            && prefix.is_none_or(|p| {
                self.imports()
                    .iter()
                    .any(|(m, a)| *m == "Test" && (*m == p || a.as_deref() == Some(p.as_str())))
            })
    }

    /// The module's imports: each module's name, and its alias.
    fn imports(&self) -> Vec<(String, Option<String>)> {
        (self.module.root().children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .filter_map(|i| {
                let name = words(&child(&i, SyntaxKind::ModuleName)?);
                let alias = child(&i, SyntaxKind::ImportAlias)
                    .and_then(|a| crate::symbols::token(&a, SyntaxKind::UpperIdent));
                Some((name, alias))
            })
            .collect()
    }

    /// `getQualifierForName`: none where an import exposes `name`, else
    /// the alias or name of the module's last import.
    fn qualifier(&self, module: &str, name: &str) -> String {
        let exposes = (self.module.root().children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .filter_map(|i| child(&i, SyntaxKind::ExposingList))
            .any(|list| {
                list.children_with_tokens()
                    .any(|e| e.kind() == SyntaxKind::DotDot)
                    || list
                        .children()
                        .any(|n| n.kind() == SyntaxKind::ExposedValue && words(&n) == name)
            });
        if exposes {
            return String::new();
        }
        match self.imports().into_iter().rfind(|(m, _)| m == module) {
            Some((m, alias)) => format!("{}.", alias.unwrap_or(m)),
            None => String::new(),
        }
    }
}

fn is_test(ty: &error::Type) -> bool {
    match ty {
        error::Type::Type(home, name, _) => &*home.module == "Test.Internal" && name == "Test",
        error::Type::Alias(_, _, _, real) => is_test(real),
        _ => false,
    }
}

fn is_string(ty: &error::Type) -> bool {
    match ty {
        error::Type::Type(home, name, _) => &*home.module == "String" && name == "String",
        error::Type::Alias(_, _, _, real) => is_string(real),
        _ => false,
    }
}

/// `stringLiteralToLabel`: a triple-quoted string as written inside, any
/// other unescaped as JSON reads it.
fn label_of(literal: &str) -> String {
    if let Some(inner) = literal
        .strip_prefix("\"\"\"")
        .and_then(|l| l.strip_suffix("\"\"\""))
    {
        return inner.to_owned();
    }
    serde_json_unescape(literal).unwrap_or_else(|| literal.trim_matches('"').to_owned())
}

/// `JSON.parse` of a string literal: its escapes, as JSON has them.
fn serde_json_unescape(literal: &str) -> Option<String> {
    let inner = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            // JSON rejects any other escape, as Elm's `\u{…}`.
            _ => return None,
        }
    }
    Some(out)
}
