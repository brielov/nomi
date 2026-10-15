//! The names `Nomi.Ffi.task` calls an FFI module's functions by: what the
//! module exports, read by a lexical scan of its source; the calls a
//! program reaches whose name is a literal, checked against that; and
//! the checks the generated module makes of the rest when it starts.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::Compat;
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::{Choice, Decider, Def, Expr, Global, GlobalGraph, Main, Node};

use crate::mode::Mode;
use crate::reach::reach;

/// An entry's FFI module: the specifier it is imported by, and what it
/// exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ffi {
    pub specifier: String,
    pub exports: Exports,
}

/// An ES module's exports, as a lexical scan of its source finds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exports {
    /// The names found; a default export is `default`.
    pub names: BTreeSet<String>,
    /// Whether `names` are all of them: not when the module re-exports
    /// all of another's, or the scan met what it doesn't follow.
    pub complete: bool,
}

/// What a module checks of its FFI module when it starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Startup {
    /// The literal names called that the scan found no export of, while
    /// not finding all, each with a definition calling it.
    pub names: BTreeMap<String, Global>,
    /// Whether a call's name isn't a literal, so every export has to be a
    /// function for whatever name it makes.
    pub only_functions: bool,
}

/// A call whose literal name the FFI module doesn't export: the
/// definition making it, and the name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Misspelt {
    pub caller: Global,
    pub name: String,
}

/// The `Nomi.Ffi.task` calls `mains` reach in `graph`, written in `mode`,
/// checked against `exports`: the calls whose literal name it lacks, or
/// what the module is left to check when it starts. A call's name is a
/// literal when its first argument is a string without escapes, partial
/// applications included; a name passed through a helper or a pipeline
/// into `task` is not.
pub fn check(
    compat: Compat,
    mode: &Mode,
    exports: &Exports,
    graph: &GlobalGraph,
    mains: &[(Home, Main)],
) -> Result<Startup, Vec<Misspelt>> {
    let mut calls = Calls::default();
    for global in reach(compat, mode.is_debug(), graph, mains).order {
        match &graph.nodes[global] {
            Node::Define(expr, _) | Node::PortIncoming(expr, _) | Node::PortOutgoing(expr, _) => {
                calls.expr(global, expr);
            }
            Node::DefineTailFunc { body, .. } => calls.expr(global, body),
            Node::Cycle {
                values, functions, ..
            } => {
                for (_, expr) in values {
                    calls.expr(global, expr);
                }
                for def in functions {
                    calls.def(global, def);
                }
            }
            Node::Link(linked) => calls.dynamic |= is_task(linked),
            Node::Ctor(..) | Node::Enum(_) | Node::Box | Node::Manager(_) | Node::Kernel(..) => {}
        }
    }
    let mut startup = Startup {
        names: BTreeMap::new(),
        only_functions: calls.dynamic,
    };
    let mut misspelt = BTreeSet::new();
    for (caller, name) in calls.literal {
        if exports.names.contains(name) {
            continue;
        }
        if exports.complete {
            misspelt.insert(Misspelt {
                caller: caller.clone(),
                name: name.to_owned(),
            });
        } else {
            (startup.names)
                .entry(name.to_owned())
                .or_insert_with(|| caller.clone());
        }
    }
    if misspelt.is_empty() {
        Ok(startup)
    } else {
        Err(misspelt.into_iter().collect())
    }
}

/// Whether `global` is `Nomi.Ffi.task`.
fn is_task(global: &Global) -> bool {
    global.name == "task" && global.home.package == "nomi/core" && global.home.module == "Nomi.Ffi"
}

/// The calls found: those with a literal name, each with the definition
/// making it, and whether `task` is used any other way.
#[derive(Default)]
struct Calls<'a> {
    literal: Vec<(&'a Global, &'a str)>,
    dynamic: bool,
}

impl<'a> Calls<'a> {
    fn def(&mut self, caller: &'a Global, def: &'a Def) {
        match def {
            Def::Def(_, body) | Def::TailDef { body, .. } => self.expr(caller, body),
        }
    }

    fn expr(&mut self, caller: &'a Global, expr: &'a Expr) {
        nomi_base::grow(|| match expr {
            Expr::Call(f, args) => {
                match (&**f, args.first()) {
                    (Expr::VarGlobal(global), Some(Expr::Str(name)))
                        if is_task(global) && !name.contains('\\') =>
                    {
                        self.literal.push((caller, name));
                    }
                    _ => self.expr(caller, f),
                }
                for arg in args {
                    self.expr(caller, arg);
                }
            }
            Expr::VarGlobal(global) => self.dynamic |= is_task(global),
            Expr::Bool(_)
            | Expr::Chr(_)
            | Expr::Str(_)
            | Expr::Int(_)
            | Expr::Float(_)
            | Expr::VarLocal(_)
            | Expr::VarEnum(..)
            | Expr::VarBox(_)
            | Expr::VarCycle(..)
            | Expr::VarDebug { .. }
            | Expr::VarKernel { .. }
            | Expr::Accessor(_)
            | Expr::Unit
            | Expr::Shader { .. } => {}
            Expr::List(items) => {
                for item in items {
                    self.expr(caller, item);
                }
            }
            Expr::Function(_, body) | Expr::Destruct(_, body) | Expr::Access(body, _) => {
                self.expr(caller, body);
            }
            Expr::TailCall(_, args) => {
                for (_, arg) in args {
                    self.expr(caller, arg);
                }
            }
            Expr::If(branches, last) => {
                for (condition, branch) in branches {
                    self.expr(caller, condition);
                    self.expr(caller, branch);
                }
                self.expr(caller, last);
            }
            Expr::Let(def, body) => {
                self.def(caller, def);
                self.expr(caller, body);
            }
            Expr::Case { decider, jumps, .. } => {
                self.decider(caller, decider);
                for (_, jump) in jumps {
                    self.expr(caller, jump);
                }
            }
            Expr::Update(record, fields) => {
                self.expr(caller, record);
                for field in fields.values() {
                    self.expr(caller, field);
                }
            }
            Expr::Record(fields) => {
                for field in fields.values() {
                    self.expr(caller, field);
                }
            }
            Expr::Tuple(a, b, c) => {
                self.expr(caller, a);
                self.expr(caller, b);
                if let Some(c) = c {
                    self.expr(caller, c);
                }
            }
        });
    }

    fn decider(&mut self, caller: &'a Global, decider: &'a Decider<Choice>) {
        nomi_base::grow(|| match decider {
            Decider::Leaf(Choice::Inline(expr)) => self.expr(caller, expr),
            Decider::Leaf(Choice::Jump(_)) => {}
            Decider::Chain {
                success, failure, ..
            } => {
                self.decider(caller, success);
                self.decider(caller, failure);
            }
            Decider::FanOut {
                tests, fallback, ..
            } => {
                for (_, decider) in tests {
                    self.decider(caller, decider);
                }
                self.decider(caller, fallback);
            }
        });
    }
}

impl Startup {
    /// The statements that check, in the scope `ffi` is imported into,
    /// what `self` asks, throwing at the first export missing or not a
    /// function.
    pub(crate) fn js(&self, ffi: &str) -> String {
        let mut js = String::new();
        if !self.names.is_empty() {
            let names = (self.names.iter())
                .map(|(name, caller)| {
                    format!("[\"{name}\", \"{}.{}\"]", caller.home.module, caller.name)
                })
                .collect::<Vec<_>>()
                .join(", ");
            js.push_str(&format!(
                "for (const [name, caller] of [{names}]) if (typeof {ffi}[name] !== 'function') \
                 throw new Error('the FFI module exports no function `' + name + '`, which ' + \
                 caller + ' calls');\n"
            ));
        }
        if self.only_functions {
            js.push_str(&format!(
                "for (const name of Object.keys({ffi})) if (typeof {ffi}[name] !== 'function') \
                 throw new Error('the FFI module exports `' + name + '`, which is not a \
                 function, and Nomi.Ffi.task calls exports by names made at run time');\n"
            ));
        }
        js
    }
}

/// `exports`: what the ES module `source` exports, by a lexical scan of
/// its top level: declarations of functions, classes and single
/// variables, lists of names, default exports and namespaces re-exported
/// under a name. Anything else leaves the scan incomplete.
pub fn exports(source: &str) -> Exports {
    let Some(tokens) = tokens(source) else {
        return Exports::default();
    };
    let mut exports = Exports {
        names: BTreeSet::new(),
        complete: true,
    };
    let mut cursor = Cursor {
        tokens: &tokens,
        at: 0,
    };
    while let Some(&(token, depth)) = tokens.get(cursor.at) {
        let after_dot = cursor.at > 0 && tokens[cursor.at - 1].0 == Token::Punct(b'.');
        cursor.at += 1;
        if depth != 0 || token != Token::Word("export") || after_dot {
            continue;
        }
        match export(&mut cursor) {
            Some(names) => exports.names.extend(names.into_iter().map(str::to_owned)),
            None => exports.complete = false,
        }
    }
    exports
}

/// The names of the export at `cursor`, past its `export`; `None` when
/// the scan doesn't follow it.
fn export<'a>(cursor: &mut Cursor<'a, '_>) -> Option<Vec<&'a str>> {
    match cursor.next()? {
        Token::Word("default") => Some(vec!["default"]),
        Token::Word("async") => {
            (cursor.word()? == "function").then_some(())?;
            Some(vec![cursor.function()?])
        }
        Token::Word("function") => Some(vec![cursor.function()?]),
        Token::Word("class") => Some(vec![cursor.word()?]),
        Token::Word("const" | "let" | "var") => {
            let name = cursor.word()?;
            // One declarator: a comma before the statement ends may
            // declare more.
            while let Some(&(token, depth)) = cursor.tokens.get(cursor.at) {
                match token {
                    _ if depth != 0 => {}
                    Token::Punct(b';') | Token::Word("export") => break,
                    Token::Punct(b',') => return None,
                    _ => {}
                }
                cursor.at += 1;
            }
            Some(vec![name])
        }
        Token::Punct(b'*') => {
            (cursor.word()? == "as").then_some(())?;
            Some(vec![cursor.name()?])
        }
        Token::Punct(b'{') => {
            let mut names = Vec::new();
            if cursor.peek() == Some(Token::Punct(b'}')) {
                return Some(names);
            }
            loop {
                let local = cursor.name()?;
                let name = if cursor.peek() == Some(Token::Word("as")) {
                    cursor.at += 1;
                    cursor.name()?
                } else {
                    local
                };
                names.push(name);
                match cursor.next()? {
                    Token::Punct(b',') if cursor.peek() == Some(Token::Punct(b'}')) => {
                        cursor.at += 1;
                        return Some(names);
                    }
                    Token::Punct(b',') => {}
                    Token::Punct(b'}') => return Some(names),
                    _ => return None,
                }
            }
        }
        _ => None,
    }
}

/// A place in a module's tokens.
struct Cursor<'a, 't> {
    tokens: &'t [(Token<'a>, usize)],
    at: usize,
}

impl<'a> Cursor<'a, '_> {
    fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.at).map(|(token, _)| *token)
    }

    fn next(&mut self) -> Option<Token<'a>> {
        let token = self.peek();
        self.at += 1;
        token
    }

    fn word(&mut self) -> Option<&'a str> {
        match self.next()? {
            Token::Word(word) => Some(word),
            _ => None,
        }
    }

    /// A name in a list of exports: a word or a string.
    fn name(&mut self) -> Option<&'a str> {
        match self.next()? {
            Token::Word(name) | Token::Str(name) => Some(name),
            _ => None,
        }
    }

    /// A function's name, past `function` and any `*`.
    fn function(&mut self) -> Option<&'a str> {
        if self.peek() == Some(Token::Punct(b'*')) {
            self.at += 1;
        }
        self.word()
    }
}

/// A token of JavaScript, as far as the scan tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token<'a> {
    /// An identifier or keyword.
    Word(&'a str),
    /// A string literal's text, without escapes.
    Str(&'a str),
    Punct(u8),
    /// A number, a regular expression, a template, or a string with
    /// escapes: what a division may follow.
    Value,
}

/// What a bracket opens: a bracket's match, or a template's text after a
/// substitution's `}`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Bracket(u8),
    Substitution,
}

/// `source`'s tokens, each with how many brackets enclose it; `None` when
/// a comment, string, template or regular expression doesn't end or
/// brackets don't match.
fn tokens(source: &str) -> Option<Vec<(Token<'_>, usize)>> {
    let bytes = source.as_bytes();
    let mut at = match source.strip_prefix("#!") {
        Some(_) => source.find('\n').unwrap_or(source.len()),
        None => 0,
    };
    let mut tokens: Vec<(Token<'_>, usize)> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    while let Some(&byte) = bytes.get(at) {
        let depth = open.len();
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' => at += 1,
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                at = source[at..].find('\n').map_or(source.len(), |end| at + end);
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += 2 + source[at + 2..].find("*/")? + 2;
            }
            b'/' if !divides(tokens.last().map(|(token, _)| *token)) => {
                at = regex(bytes, at + 1)?;
                tokens.push((Token::Value, depth));
            }
            b'\'' | b'"' => {
                let end = string(bytes, at + 1, byte)?;
                let text = &source[at + 1..end];
                let token = if text.contains('\\') {
                    Token::Value
                } else {
                    Token::Str(text)
                };
                tokens.push((token, depth));
                at = end + 1;
            }
            b'`' => {
                let (end, substitution) = template(bytes, at + 1)?;
                tokens.push((Token::Value, depth));
                at = end;
                if substitution {
                    open.push(Open::Substitution);
                }
            }
            b'0'..=b'9' => {
                while bytes
                    .get(at)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'.')
                {
                    at += 1;
                }
                tokens.push((Token::Value, depth));
            }
            b'(' | b'[' | b'{' => {
                tokens.push((Token::Punct(byte), depth));
                open.push(Open::Bracket(byte));
                at += 1;
            }
            b')' | b']' | b'}' => match (open.pop()?, byte) {
                (Open::Bracket(b'('), b')')
                | (Open::Bracket(b'['), b']')
                | (Open::Bracket(b'{'), b'}') => {
                    tokens.push((Token::Punct(byte), depth - 1));
                    at += 1;
                }
                (Open::Substitution, b'}') => {
                    let (end, substitution) = template(bytes, at + 1)?;
                    tokens.push((Token::Value, depth - 1));
                    at = end;
                    if substitution {
                        open.push(Open::Substitution);
                    }
                }
                _ => return None,
            },
            _ if is_word(byte) => {
                let start = at;
                while bytes
                    .get(at)
                    .is_some_and(|b| is_word(*b) || b.is_ascii_digit())
                {
                    at += 1;
                }
                tokens.push((Token::Word(&source[start..at]), depth));
            }
            _ => {
                tokens.push((Token::Punct(byte), depth));
                at += 1;
            }
        }
    }
    open.is_empty().then_some(tokens)
}

/// Whether a byte may start an identifier; any byte of a character
/// outside ASCII counts.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// Whether a `/` after `before` divides, rather than starting a regular
/// expression.
fn divides(before: Option<Token<'_>>) -> bool {
    match before {
        Some(Token::Word(word)) => !matches!(
            word,
            "return"
                | "typeof"
                | "instanceof"
                | "in"
                | "of"
                | "new"
                | "delete"
                | "void"
                | "throw"
                | "case"
                | "do"
                | "else"
                | "yield"
                | "await"
        ),
        Some(Token::Str(_) | Token::Value | Token::Punct(b')' | b']' | b'}')) => true,
        Some(Token::Punct(_)) | None => false,
    }
}

/// Where the string quoted by `quote` from `at` ends: its closing quote.
fn string(bytes: &[u8], mut at: usize, quote: u8) -> Option<usize> {
    loop {
        match *bytes.get(at)? {
            b'\\' => at += 2,
            b'\n' => return None,
            byte if byte == quote => return Some(at),
            _ => at += 1,
        }
    }
}

/// Past the end of the regular expression from `at`, its flags included.
fn regex(bytes: &[u8], mut at: usize) -> Option<usize> {
    let mut class = false;
    loop {
        match *bytes.get(at)? {
            b'\\' => at += 1,
            b'\n' => return None,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => break,
            _ => {}
        }
        at += 1;
    }
    at += 1;
    while bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
        at += 1;
    }
    Some(at)
}

/// Where a template's text from `at` ends, past its closing backquote or
/// a substitution's `${`, and whether a substitution follows.
fn template(bytes: &[u8], mut at: usize) -> Option<(usize, bool)> {
    loop {
        match *bytes.get(at)? {
            b'\\' => at += 2,
            b'`' => return Some((at + 1, false)),
            b'$' if bytes.get(at + 1) == Some(&b'{') => return Some((at + 2, true)),
            _ => at += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(module: &str, name: &str) -> Global {
        Global {
            home: Home::author_project(module),
            name: name.to_owned(),
        }
    }

    fn task() -> Global {
        Global {
            home: Home {
                package: "nomi/core".into(),
                module: "Nomi.Ffi".into(),
            },
            name: "task".to_owned(),
        }
    }

    fn call(name: Expr) -> Expr {
        Expr::Call(Box::new(Expr::VarGlobal(task())), vec![name, Expr::Unit])
    }

    fn define(expr: Expr, deps: &[Global]) -> Node {
        Node::Define(expr, deps.iter().cloned().collect())
    }

    /// `Main.main` calls `add` and, partially applied, `ad`, and reaches
    /// `Page.load` when `dynamic`, which calls by an escaped literal and
    /// by a parameter; `Page.unused`, which nothing reaches, calls `nope`.
    fn graph(dynamic: bool) -> GlobalGraph {
        let str = |name: &str| Expr::Str(name.to_owned());
        let mut main = vec![
            call(str("add")),
            Expr::Call(Box::new(Expr::VarGlobal(task())), vec![str("ad")]),
        ];
        let mut deps = vec![task()];
        if dynamic {
            main.push(Expr::VarGlobal(global("Page", "load")));
            deps.push(global("Page", "load"));
        }
        let load = Expr::Tuple(
            Box::new(call(str("a\\u{0064}d"))),
            Box::new(Expr::Function(
                vec!["name".to_owned()],
                Box::new(call(Expr::VarLocal("name".to_owned()))),
            )),
            None,
        );
        let mut graph = GlobalGraph::default();
        graph.nodes.extend([
            (global("Main", "main"), define(Expr::List(main), &deps)),
            (global("Page", "load"), define(load, &[task()])),
            (
                global("Page", "unused"),
                define(call(str("nope")), &[task()]),
            ),
            (task(), define(Expr::Unit, &[])),
        ]);
        graph
    }

    #[test]
    fn literal_names_are_checked_and_the_rest_left_to_startup() {
        let mains = [(Home::author_project("Main"), Main::Static)];
        let dev = Mode::Dev { debug: None };
        let check = |dynamic, complete| {
            let exports = Exports {
                names: ["add".to_owned()].into(),
                complete,
            };
            check(Compat::V0_19_1, &dev, &exports, &graph(dynamic), &mains)
        };
        let misspelt = Err(vec![Misspelt {
            caller: global("Main", "main"),
            name: "ad".to_owned(),
        }]);
        assert_eq!(check(true, true), misspelt);
        assert_eq!(check(false, true), misspelt);
        assert_eq!(
            check(true, false),
            Ok(Startup {
                names: [("ad".to_owned(), global("Main", "main"))].into(),
                only_functions: true,
            })
        );
        assert_eq!(
            check(false, false),
            Ok(Startup {
                names: [("ad".to_owned(), global("Main", "main"))].into(),
                only_functions: false,
            })
        );
    }

    /// What a scan of `source` finds, in order, and whether it's all.
    fn found(source: &str) -> (Vec<String>, bool) {
        let exports = exports(source);
        (exports.names.into_iter().collect(), exports.complete)
    }

    fn all(names: &[&str]) -> (Vec<String>, bool) {
        (names.iter().map(|name| (*name).to_owned()).collect(), true)
    }

    #[test]
    fn declarations_and_lists_of_names_are_exports() {
        assert_eq!(
            found(
                "#!/usr/bin/env node\n\
                 export async function add(a, b) { return a + b; }\n\
                 export function sync() {}\n\
                 export async function* stream() {}\n\
                 export function* gen() {}\n\
                 export const half = (x, y) => x / 2;\n\
                 export let counter = 0\n\
                 export var old = function (a, b) { return [a, b]; }\n\
                 export class Thing {}\n\
                 const a = 1, b = 2;\n\
                 export { a, b as c, a as \"string name\", };\n\
                 export { x as y } from './other.mjs';\n\
                 export * as all from './other.mjs';\n\
                 export default function named() {}\n"
            ),
            all(&[
                "Thing",
                "a",
                "add",
                "all",
                "c",
                "counter",
                "default",
                "gen",
                "half",
                "old",
                "stream",
                "string name",
                "sync",
                "y",
            ])
        );
        assert_eq!(found("export {};\n"), all(&[]));
    }

    #[test]
    fn exports_are_found_only_at_the_top_level() {
        assert_eq!(
            found(
                "// export function commented() {}\n\
                 /* export function blocked() {} */\n\
                 const s = 'export function quoted() {}', t = \"{\";\n\
                 const u = `export ${ { a: `${'}'}` }.a } function templated() {}`;\n\
                 const r = /export function [}]matched\\/ {/g, d = 4 / 2 / 1;\n\
                 function f() { return { export: 1 }; }\n\
                 f().export;\n\
                 export function kept() {}\n"
            ),
            all(&["kept"])
        );
    }

    #[test]
    fn what_the_scan_cannot_follow_leaves_it_incomplete() {
        let incomplete = |source: &str| !found(source).1;
        assert!(incomplete(
            "export * from './impl.mjs';\nexport function f() {}\n"
        ));
        assert!(incomplete("export const a = 1, b = 2;\n"));
        assert!(incomplete("export const { a, b } = obj;\n"));
        assert!(incomplete("export { a as 'b\\x' };\n"));
        assert!(incomplete("export function f() {\n"));
        assert!(incomplete("export function f() {}\nconst s = 'open\n"));
        assert!(incomplete("export function f() {}\n/* open\n"));
        assert!(incomplete("export function f() { ]\n"));
        // What it can still name, it does.
        assert_eq!(
            found("export * from './impl.mjs';\nexport function f() {}\n"),
            (vec!["f".to_owned()], false)
        );
    }
}
