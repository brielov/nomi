//! The test suites of a project's `tests/` modules, as elm-language-server
//! finds them for test explorers (`findTestsProvider.ts`): each top-level
//! value whose expression is a call that makes a `Test`, a suite where it
//! is `describe` with a list of tests, each labelled by its one string.

use std::path::{Path, PathBuf};

use nomi_base::TextSize;
use nomi_compiler::types::{ExprTypes, error};
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, literal_body};

use crate::tree::{WrittenImport, child, code_range, words, written_imports};

/// A test or a suite of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestSuite {
    pub label: String,
    pub path: PathBuf,
    /// Where its call starts.
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
        let top: Vec<TestSuite> = (finder.root.children())
            .filter(|n| n.kind() == SyntaxKind::ValueDecl)
            .filter_map(|decl| child(&decl, SyntaxKind::Definition))
            .filter_map(|definition| finder.suite(&finder.test_call(&definition)?))
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

/// One module's tests, found by its types and imports.
struct Finder<'a> {
    module: &'a Indexed,
    root: SyntaxNode,
    types: Option<&'a ExprTypes>,
    imports: Vec<WrittenImport>,
    /// `Test.describe` as the module writes it.
    describe: String,
}

impl<'a> Finder<'a> {
    fn new(index: &Index, module: &'a Indexed) -> Finder<'a> {
        let root = module.root();
        let imports: Vec<WrittenImport> = written_imports(&root).collect();
        // `getQualifierForName`: none where an import exposes `describe`,
        // else the alias or name of the module's last import of `Test`.
        let exposed = (root.children().filter_map(ast::Import::cast))
            .filter_map(|import| import.exposing_list())
            .any(|list| {
                list.dot_dot_token().is_some()
                    || (list.syntax().children())
                        .any(|n| n.kind() == SyntaxKind::ExposedValue && words(&n) == "describe")
            });
        let describe = match imports.iter().rfind(|i| i.module == "Test") {
            Some(test) if !exposed => {
                format!("{}.describe", test.alias.as_deref().unwrap_or(&test.module))
            }
            _ => "describe".to_owned(),
        };
        Finder {
            module,
            root,
            types: module.types(index.compat).map(|(_, types)| types),
            imports,
            describe,
        }
    }

    fn type_of(&self, node: &SyntaxNode) -> Option<&error::Type> {
        crate::hover::type_of(&self.root, self.types?, code_range(node)?)
    }

    /// `findTestFunctionCall`: through `let`s, the first call in `node`,
    /// if it makes a `Test` or a function to one.
    fn test_call(&self, node: &SyntaxNode) -> Option<SyntaxNode> {
        let body = match node.kind() {
            SyntaxKind::Definition => node
                .children()
                .filter(|c| !ast::Pattern::can_cast(c.kind()))
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
        let test = |ty| is_named(ty, "Test.Internal", "Test");
        (test(ty) || test(returned)).then_some(call)
    }

    /// `findTestSuite`: a call labelled by one string literal, a suite
    /// where it's `describe` with tests.
    fn suite(&self, call: &SyntaxNode) -> Option<TestSuite> {
        let mut parts = call.children();
        let target = parts.next()?;
        let args: Vec<SyntaxNode> = parts.collect();
        let string = args.iter().find(|a| {
            self.type_of(a)
                .is_some_and(|ty| is_named(ty, "String", "String"))
        })?;
        let literals: Vec<String> = string
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::LiteralExpr)
            .filter_map(|n| {
                (n.children_with_tokens())
                    .filter_map(SyntaxElement::into_token)
                    .find(|t| !t.kind().is_trivia())
            })
            .filter(|t| matches!(t.kind(), SyntaxKind::String | SyntaxKind::MultilineString))
            .map(|t| label_of(t.text()))
            .collect();
        let [label] = literals.as_slice() else {
            return None;
        };
        let at = code_range(call)?.start();
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
            .filter_map(|call| self.suite(&call))
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
        match name.rsplit_once('.') {
            None => name == "describe",
            Some((prefix, _)) => {
                name == self.describe
                    && (self.imports.iter()).any(|i| {
                        i.module == "Test"
                            && (i.module == prefix || i.alias.as_deref() == Some(prefix))
                    })
            }
        }
    }
}

/// Whether `ty` is the type `name` of the module `module`, or an alias
/// of it.
fn is_named(ty: &error::Type, module: &str, name: &str) -> bool {
    use error::Type as E;
    match ty {
        E::Type(home, n, _) => &*home.module == module && n == name,
        E::Alias(_, _, _, real) => is_named(real, module, name),
        E::Lambda(..)
        | E::Infinite
        | E::Error
        | E::FlexVar(_)
        | E::FlexSuper(..)
        | E::RigidVar(_)
        | E::RigidSuper(..)
        | E::Record(..)
        | E::Unit
        | E::Tuple(..) => false,
    }
}

/// `stringLiteralToLabel`: a triple-quoted string as written inside, any
/// other unescaped as JSON reads it.
fn label_of(literal: &str) -> String {
    if literal.starts_with("\"\"\"")
        && let Some(body) = literal_body(literal)
    {
        return body.to_owned();
    }
    json_unescape(literal).unwrap_or_else(|| literal.trim_matches('"').to_owned())
}

/// `JSON.parse` of a string literal: its escapes, as JSON has them.
fn json_unescape(literal: &str) -> Option<String> {
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
