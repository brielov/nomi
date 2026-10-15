//! A module's shape: its HIR as elm reads it, without IDs, source
//! positions or spelling. Formatting must leave it unchanged,
//! so two spellings of one program have one shape:
//!
//! - parentheses are already gone, and `(f a) b` is `f a b`;
//! - literals are their values as elm decodes them, a negated number
//!   included, as an integer too large for 64 bits wraps to one;
//! - `(x) = e` is `x = e`;
//! - imports are each module's, once, with the qualifiers the module
//!   uses and everything they expose;
//! - exposing lists are sets, and one naming everything the module
//!   declares is `(..)`, as a module without a header is `Main` exposing
//!   everything.
//!
//! Doc comments count by presence, since their Markdown is reprinted; the
//! formatter's own invariants check their text.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use nomi_base::grow_large;
use nomi_syntax::literal_body;

use super::{
    Def, DefId, Escape, Exposed, Exposing, Expr, ExprId, Import, Module, ModuleKind, PatId,
    Pattern, Spanned, Type, TypeId, escape, int_value, plain_code,
};

/// `module`'s shape, a node a line, each before its children.
fn shape(module: &Module) -> Vec<String> {
    let mut shape = Shape {
        module,
        lines: Vec::new(),
    };
    shape.module();
    shape.lines
}

/// Where `before` and `after` first differ in shape, with the lines
/// around it; `None` when they don't.
pub fn difference(before: &Module, after: &Module) -> Option<String> {
    let (before, after) = (shape(before), shape(after));
    let at = (0..before.len().max(after.len())).find(|&i| before.get(i) != after.get(i))?;
    let near = |lines: &[String]| {
        let end = (at + 3).min(lines.len());
        lines[at.saturating_sub(3).min(end)..end].join(" | ")
    };
    // The declaration the difference is in, to find it by.
    let decl = before[..at.min(before.len())]
        .iter()
        .rev()
        .find(|l| l.starts_with("decl "))
        .map_or("the header", |l| l.as_str());
    Some(format!(
        "in {decl}, line {at}: {} became {}",
        near(&before),
        near(&after)
    ))
}

struct Shape<'m> {
    module: &'m Module,
    lines: Vec<String>,
}

impl Shape<'_> {
    fn line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    fn module(&mut self) {
        let m = self.module;
        self.line(format!("module {}", m.name()));
        match &m.kind {
            ModuleKind::Plain => self.line("plain"),
            ModuleKind::Port => self.line("port"),
            ModuleKind::Effect(manager) => {
                let fields: Vec<String> = (manager.given())
                    .map(|(key, value)| format!("{key} = {}", value.text))
                    .collect();
                self.line(format!("effect {}", fields.join(", ")));
            }
        }
        self.line(format!("exports {}", exports(m)));
        self.line(format!("docs {}", m.docs.is_some()));
        for import in imports(&m.imports, &qualifiers(m)) {
            self.line(import);
        }
        for union in &m.unions {
            self.line(format!(
                "decl type {} {} docs {}",
                union.name.text,
                texts(&union.params),
                union.docs.is_some()
            ));
            for (name, args) in &union.variants {
                self.line(format!("variant {} {}", name.text, args.len()));
                for &arg in args {
                    self.ty(arg);
                }
            }
        }
        for alias in &m.aliases {
            self.line(format!(
                "decl alias {} {} docs {}",
                alias.name.text,
                texts(&alias.params),
                alias.docs.is_some()
            ));
            self.ty(alias.body);
        }
        for infix in &m.binops {
            self.line(format!(
                "decl infix {:?} {} ({}) = {}",
                infix.fixity.associativity,
                infix.fixity.precedence,
                infix.op.text,
                infix.function.text
            ));
        }
        for port in &m.ports {
            self.line(format!(
                "decl port {} docs {}",
                port.name.text,
                port.docs.is_some()
            ));
            self.ty(port.ty);
        }
        for &value in &m.values {
            self.def(value, "decl ");
        }
    }

    fn def(&mut self, id: DefId, prefix: &str) {
        match &self.module.defs[id] {
            Def::Define {
                name,
                params,
                body,
                annotation,
                docs,
            } => {
                self.line(format!(
                    "{prefix}value {} {} annotated {} docs {}",
                    name.text,
                    params.len(),
                    annotation.is_some(),
                    docs.is_some()
                ));
                if let Some(annotation) = annotation {
                    self.ty(*annotation);
                }
                for &param in params {
                    self.pattern(param);
                }
                self.expr(*body);
            }
            Def::Destruct { pattern, body } => {
                // `(x) = e` is `x = e`.
                if let Pattern::Var(name) = &self.module.patterns[*pattern] {
                    self.line(format!("{prefix}value {name} 0 annotated false docs false"));
                } else {
                    self.line(format!("{prefix}destruct"));
                    self.pattern(*pattern);
                }
                self.expr(*body);
            }
        }
    }

    fn expr(&mut self, id: ExprId) {
        grow_large(|| self.expr_(id));
    }

    fn expr_(&mut self, id: ExprId) {
        let exprs = |s: &mut Self, ids: &[ExprId]| ids.iter().for_each(|&e| s.expr(e));
        match &self.module.exprs[id] {
            Expr::Missing => self.line("missing"),
            Expr::Chr(text) => self.line(format!("char {}", literal(text))),
            Expr::Str(text) => self.line(format!("string {}", literal(text))),
            Expr::Int(text) => self.line(format!("int {}", int_value(text))),
            Expr::Float(text) => self.line(format!("float {}", float(text))),
            Expr::Var {
                qualifier, name, ..
            } => self.line(format!("var {}", qualified(qualifier.as_deref(), name))),
            Expr::List(items) => {
                self.line(format!("list {}", items.len()));
                exprs(self, items);
            }
            Expr::Op(op) => self.line(format!("op ({op})")),
            // `-1` is the value -1, which a literal can spell too, as an
            // integer too large for 64 bits wraps to it.
            Expr::Negate(e) => match &self.module.exprs[*e] {
                Expr::Int(text) => self.line(format!("int {}", int_value(text).wrapping_neg())),
                Expr::Float(text) => self.line(format!("float {}", float(&format!("-{text}")))),
                _ => {
                    self.line("negate");
                    self.expr(*e);
                }
            },
            Expr::Binops { operands, ops } => {
                self.line(format!("binops {}", texts(ops)));
                exprs(self, operands);
            }
            Expr::Lambda { params, body } => {
                self.line(format!("lambda {}", params.len()));
                for &p in params {
                    self.pattern(p);
                }
                self.expr(*body);
            }
            Expr::Call { function, args } => self.call(*function, args),
            Expr::If {
                branches,
                otherwise,
            } => {
                self.line(format!("if {}", branches.len()));
                for &(condition, then) in branches {
                    self.expr(condition);
                    self.expr(then);
                }
                self.expr(*otherwise);
            }
            Expr::Let { defs, body } => {
                self.line(format!("let {}", defs.len()));
                for &d in defs {
                    self.def(d, "");
                }
                self.expr(*body);
            }
            Expr::Case {
                scrutinee,
                branches,
            } => {
                self.line(format!("case {}", branches.len()));
                self.expr(*scrutinee);
                for &(pattern, body) in branches {
                    self.pattern(pattern);
                    self.expr(body);
                }
            }
            Expr::Accessor(field) => self.line(format!("accessor .{field}")),
            Expr::Access { record, field } => {
                self.line(format!("access .{}", field.text));
                self.expr(*record);
            }
            Expr::Update { record, fields } => {
                self.line(format!("update {} {}", record.text, field_names(fields)));
                for &(_, e) in fields {
                    self.expr(e);
                }
            }
            Expr::Record(fields) => {
                self.line(format!("record {}", field_names(fields)));
                for &(_, e) in fields {
                    self.expr(e);
                }
            }
            Expr::Unit => self.line("unit"),
            Expr::Tuple(items) => {
                self.line(format!("tuple {}", items.len()));
                exprs(self, items);
            }
            Expr::Shader(source) => self.line(format!("shader {source:?}")),
        }
    }

    /// A call, `(f a) b` as `f a b`.
    fn call(&mut self, mut function: ExprId, args: &[ExprId]) {
        let module = self.module;
        let mut spine = vec![args];
        while let Expr::Call { function: f, args } = &module.exprs[function] {
            function = *f;
            spine.push(args);
        }
        let count: usize = spine.iter().map(|args| args.len()).sum();
        self.line(format!("call {count}"));
        self.expr(function);
        for &e in spine.into_iter().rev().flatten() {
            self.expr(e);
        }
    }

    fn pattern(&mut self, id: PatId) {
        grow_large(|| self.pattern_(id));
    }

    fn pattern_(&mut self, id: PatId) {
        let patterns = |s: &mut Self, ids: &[PatId]| ids.iter().for_each(|&p| s.pattern(p));
        match &self.module.patterns[id] {
            Pattern::Missing => self.line("missing"),
            Pattern::Anything => self.line("_"),
            Pattern::Var(name) => self.line(format!("pvar {name}")),
            Pattern::Record(fields) => self.line(format!("precord {}", texts(fields))),
            Pattern::Alias(p, name) => {
                self.line(format!("as {}", name.text));
                self.pattern(*p);
            }
            Pattern::Unit => self.line("punit"),
            Pattern::Tuple(items) => {
                self.line(format!("ptuple {}", items.len()));
                patterns(self, items);
            }
            Pattern::Ctor {
                qualifier,
                name,
                args,
            } => {
                self.line(format!(
                    "ctor {} {}",
                    qualified(qualifier.as_deref(), &name.text),
                    args.len()
                ));
                patterns(self, args);
            }
            Pattern::List(items) => {
                self.line(format!("plist {}", items.len()));
                patterns(self, items);
            }
            Pattern::Cons(head, tail) => {
                self.line("cons");
                self.pattern(*head);
                self.pattern(*tail);
            }
            Pattern::Chr(text) => self.line(format!("pchar {}", literal(text))),
            Pattern::Str(text) => self.line(format!("pstring {}", literal(text))),
            Pattern::Int(text) => self.line(format!("pint {}", int_value(text))),
        }
    }

    fn ty(&mut self, id: TypeId) {
        grow_large(|| self.ty_(id));
    }

    fn ty_(&mut self, id: TypeId) {
        match &self.module.types[id] {
            Type::Missing => self.line("missing"),
            Type::Lambda(a, b) => {
                self.line("->");
                self.ty(*a);
                self.ty(*b);
            }
            Type::Var(name) => self.line(format!("tvar {name}")),
            Type::Named {
                qualifier,
                name,
                args,
            } => {
                self.line(format!(
                    "type {} {}",
                    qualified(qualifier.as_deref(), &name.text),
                    args.len()
                ));
                for &a in args {
                    self.ty(a);
                }
            }
            Type::Record { fields, extends } => {
                let extends = extends.as_ref().map_or("", |e| e.text.as_str());
                self.line(format!("trecord {extends} | {}", field_names(fields)));
                for &(_, t) in fields {
                    self.ty(t);
                }
            }
            Type::Unit => self.line("tunit"),
            Type::Tuple(items) => {
                self.line(format!("ttuple {}", items.len()));
                for &t in items {
                    self.ty(t);
                }
            }
        }
    }
}

fn texts<'a>(names: impl IntoIterator<Item = &'a Spanned>) -> String {
    let names: Vec<&str> = names.into_iter().map(|n| n.text.as_str()).collect();
    names.join(" ")
}

fn field_names<T>(fields: &[(Spanned, T)]) -> String {
    texts(fields.iter().map(|(name, _)| name))
}

fn qualified(qualifier: Option<&str>, name: &str) -> String {
    match qualifier {
        Some(q) => format!("{q}.{name}"),
        None => name.to_owned(),
    }
}

/// Every qualifier the module's names use.
fn qualifiers(m: &Module) -> BTreeSet<&str> {
    let exprs = m.exprs.iter().filter_map(|(_, e)| match e {
        Expr::Var { qualifier, .. } => qualifier.as_deref(),
        _ => None,
    });
    let patterns = m.patterns.iter().filter_map(|(_, p)| match p {
        Pattern::Ctor { qualifier, .. } => qualifier.as_deref(),
        _ => None,
    });
    let types = m.types.iter().filter_map(|(_, t)| match t {
        Type::Named { qualifier, .. } => qualifier.as_deref(),
        _ => None,
    });
    exprs.chain(patterns).chain(types).collect()
}

/// What a module's imports, together, give the module importing it.
#[derive(Default)]
struct Imported<'m> {
    /// The qualifiers the module uses.
    qualifiers: BTreeSet<&'m str>,
    exposing: Exposure<'m>,
}

enum Exposure<'m> {
    Everything,
    Items(Vec<&'m Exposed>),
}

impl Default for Exposure<'_> {
    fn default() -> Self {
        Exposure::Items(Vec::new())
    }
}

/// The imports as elm resolves them: each module once, with the
/// qualifiers its imports give it that the module uses, and everything
/// they expose. elm-format keeps one alias of a module imported twice,
/// which changes nothing unless the other is used.
fn imports(imports: &[Import], used: &BTreeSet<&str>) -> Vec<String> {
    let mut modules: BTreeMap<&str, Imported> = BTreeMap::new();
    for import in imports {
        let imported = modules.entry(&import.name.text).or_default();
        let qualifier = import.alias.as_ref().unwrap_or(&import.name).text.as_str();
        if used.contains(qualifier) {
            imported.qualifiers.insert(qualifier);
        }
        match (&import.exposing, &mut imported.exposing) {
            (Some(Exposing::Open), exposing) => *exposing = Exposure::Everything,
            (Some(Exposing::Explicit(exposed)), Exposure::Items(items)) => items.extend(exposed),
            (None, _) | (Some(Exposing::Explicit(_)), Exposure::Everything) => {}
        }
    }
    (modules.into_iter())
        .map(|(name, imported)| {
            let qualifiers: Vec<&str> = imported.qualifiers.into_iter().collect();
            let exposing = match imported.exposing {
                Exposure::Everything => "(..)".to_owned(),
                Exposure::Items(items) => exposed(items),
            };
            format!(
                "import {name} as {} exposing {exposing}",
                qualifiers.join(" ")
            )
        })
        .collect()
}

/// A module's exports as a set; naming everything it declares is
/// `(..)`, as elm-format writes a header for a module without one.
fn exports(m: &Module) -> String {
    let Exposing::Explicit(items) = &m.exports else {
        return "(..)".to_owned();
    };
    let named = names(items.iter());
    let values = m.value_names().map(|name| (name.text.clone(), false));
    let everything: BTreeMap<String, bool> = (values)
        .chain(m.unions.iter().map(|u| (u.name.text.clone(), true)))
        .chain(m.aliases.iter().map(|a| (a.name.text.clone(), false)))
        .chain(m.ports.iter().map(|p| (p.name.text.clone(), false)))
        .chain(m.binops.iter().map(|b| (format!("({})", b.op.text), false)))
        .collect();
    if named == everything {
        "(..)".to_owned()
    } else {
        listing(named)
    }
}

/// Exposed names as a set, a type with its variants standing for itself
/// too.
fn exposed<'e>(items: impl IntoIterator<Item = &'e Exposed>) -> String {
    listing(names(items))
}

/// Each exposed name, and whether its variants are.
fn names<'e>(items: impl IntoIterator<Item = &'e Exposed>) -> BTreeMap<String, bool> {
    let mut names: BTreeMap<String, bool> = BTreeMap::new();
    for item in items {
        let (name, open) = match item {
            Exposed::Lower(n) => (n.text.clone(), false),
            Exposed::Upper(n, variants) => (n.text.clone(), variants.is_some()),
            Exposed::Operator(n) => (format!("({})", n.text), false),
        };
        *names.entry(name).or_default() |= open;
    }
    names
}

fn listing(names: BTreeMap<String, bool>) -> String {
    let names: Vec<String> = (names.into_iter())
        .map(|(name, open)| if open { format!("{name}(..)") } else { name })
        .collect();
    format!("({})", names.join(", "))
}

/// A float literal's value, to the bit.
fn float(text: &str) -> String {
    text.parse::<f64>()
        .map_or_else(|_| format!("{text:?}"), |f| format!("{:#x}", f.to_bits()))
}

/// A character or string literal's code points.
fn literal(text: &str) -> String {
    literal_body(text).map_or_else(|| format!("{text:?}"), |b| points(&decode(b)))
}

fn points(points: &[u32]) -> String {
    let mut text = String::with_capacity(points.len());
    for &p in points {
        match char::from_u32(p) {
            Some(c) => text.push(c),
            // Writing to a `String` can't fail.
            None => _ = write!(text, "\\u{{{p:X}}}"),
        }
    }
    format!("{text:?}")
}

/// A literal's body as elm's parser decodes it (`Parse/String.hs`):
/// escapes to their code points, everything else, line breaks included,
/// as written.
fn decode(body: &str) -> Vec<u32> {
    let mut points = Vec::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        points.push(match c {
            '\\' => match escape(&mut chars) {
                Escape::Unicode(code) => code,
                Escape::Plain(c) => plain_code(c),
            },
            c => u32::from(c),
        });
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::lowered;

    fn same(a: &str, b: &str) -> Option<String> {
        difference(&lowered(a), &lowered(b))
    }

    #[test]
    fn spellings_of_one_program_have_one_shape() {
        let header = "module Main exposing (..)\n\n";
        assert_eq!(same("a = (f x) y\n", "a = f x y\n"), None);
        assert_eq!(same("a = -0x10\n", "a = -16\n"), None);
        assert_eq!(same("a = let (x) = 1 in x\n", "a = let x = 1 in x\n"), None);
        assert_eq!(same("a = 1\n", &format!("{header}a = 1\n")), None);
        assert_eq!(
            same(
                "module A exposing (a, T(..))\ntype T = T\na = 1\n",
                "module A exposing (..)\ntype T = T\na = 1\n"
            ),
            None
        );
        // `B` goes unused, so keeping one alias keeps the meaning.
        assert_eq!(
            same("import A\nimport A as B\na = A.x\n", "import A\na = A.x\n"),
            None
        );
    }

    #[test]
    fn a_change_in_meaning_shows_where() {
        let found = same("a = 1\nb = f (g x)\n", "a = 1\nb = (f g) x\n").unwrap();
        assert!(found.starts_with("in decl value b 0"), "{found}");
        assert!(same("import A as B\na = B.x\n", "import A as C\na = B.x\n").is_some());
        // One shape running out before the other.
        assert!(same("a = 1\nb = 2\n", "a = 1\n").is_some());
        assert!(same("a = 1\n", "a = 1\nb = 2\n").is_some());
    }

    #[test]
    fn literals_compare_by_value() {
        assert_eq!(int_value("0xFF"), int_value("255"));
        assert_eq!(int_value("-0x10"), -16);
        assert_eq!(int_value("9223372036854775808"), i64::MIN);
        assert_eq!(float("1e3"), float("1000.0"));
        assert_eq!(literal(r#""a\u{0041}\n""#), literal("\"\"\"aA\n\"\"\""));
        assert_ne!(literal("\"\"\"a\r\nb\"\"\""), literal("\"\"\"a\nb\"\"\""));
        assert_eq!(literal(r"'\''"), literal(r"'\u{27}'"));
        assert_eq!(literal(r"'\u{1F600}'"), literal("'😀'"));
    }
}
