//! `Optimize/Expression.hs`: canonical expressions made optimized, with
//! destructuring turned into paths and self-calls in tail position into
//! loops. Each step registers what it needs in the [`Tracker`] in
//! upstream's order, which decides the names it generates.

use std::collections::BTreeSet;

use nomi_base::diagnostic::code::Region;
use nomi_base::{Compat, TextRange};

use super::names::Tracker;
use super::{case, literal};
use crate::canonical::{self, CtorOpts, Def as CDef, ExprKind, Home, Operator, PatternKind};
use crate::optimized::{Choice, Decider, Def, Destructor, Expr, Path};
use crate::types::in_name_order;

/// What optimizing a module's expressions needs to know.
pub(super) struct Context<'a> {
    pub compat: Compat,
    /// The module's home, for `Debug` values.
    pub home: &'a Home,
    /// A canonical expression's region as rows and columns.
    pub region: &'a dyn Fn(TextRange) -> Region,
}

/// Names of the definitions of a cycle that take no arguments, which are
/// referred to through the cycle.
pub(super) type Cycle = BTreeSet<String>;

/// `optimize`.
pub(super) fn optimize(
    cx: &Context,
    tracker: &mut Tracker,
    cycle: &Cycle,
    expr: &canonical::Expr,
) -> Expr {
    nomi_base::grow(|| optimize_(cx, tracker, cycle, expr))
}

#[expect(
    clippy::too_many_lines,
    reason = "upstream's `optimize`, one short arm per expression"
)]
fn optimize_(cx: &Context, t: &mut Tracker, cycle: &Cycle, expr: &canonical::Expr) -> Expr {
    match &expr.kind {
        ExprKind::VarLocal(name) => Expr::VarLocal(name.clone()),
        ExprKind::VarTopLevel(home, name) => {
            if cycle.contains(name) {
                Expr::VarCycle(home.clone(), name.clone())
            } else {
                t.global(home, name)
            }
        }
        ExprKind::VarKernel { module, name } => {
            t.kernel(module);
            Expr::VarKernel {
                module: module.clone(),
                name: name.clone(),
            }
        }
        ExprKind::VarForeign { home, name, .. }
        | ExprKind::VarOperator(Operator {
            home,
            function: name,
            ..
        }) => t.global(home, name),
        ExprKind::VarCtor {
            opts,
            home,
            name,
            index,
            ..
        } => t.ctor(home, name, *index, *opts),
        ExprKind::VarDebug { home, name, .. } => t.debug(name, home, (cx.region)(expr.region)),
        ExprKind::Chr(token) => {
            t.kernel("Utils");
            Expr::Chr(literal::char(cx.compat, token))
        }
        ExprKind::Str(token) => Expr::Str(literal::string(cx.compat, token)),
        ExprKind::Int(n) => Expr::Int(*n),
        ExprKind::Float(text) => Expr::Float(text.clone()),
        ExprKind::List(entries) => {
            t.kernel("List");
            Expr::List(entries.iter().map(|e| optimize(cx, t, cycle, e)).collect())
        }
        ExprKind::Negate(expr) => {
            let func = t.global(&Home::core("Basics"), "negate");
            let arg = optimize(cx, t, cycle, expr);
            Expr::Call(Box::new(func), vec![arg])
        }
        ExprKind::Binop {
            operator,
            left,
            right,
        } => {
            let func = t.global(&operator.home, &operator.function);
            let left = optimize(cx, t, cycle, left);
            let right = optimize(cx, t, cycle, right);
            Expr::Call(Box::new(func), vec![left, right])
        }
        ExprKind::Lambda(args, body) => {
            let (names, destructors) = destruct_args(cx, t, args);
            let body = optimize(cx, t, cycle, body);
            Expr::Function(names, Box::new(wrap(destructors, body)))
        }
        ExprKind::Call(func, args) => {
            let func = optimize(cx, t, cycle, func);
            let args = args.iter().map(|a| optimize(cx, t, cycle, a)).collect();
            Expr::Call(Box::new(func), args)
        }
        ExprKind::If(branches, otherwise) => {
            let branches = (branches.iter())
                .map(|(c, b)| (optimize(cx, t, cycle, c), optimize(cx, t, cycle, b)))
                .collect();
            Expr::If(branches, Box::new(optimize(cx, t, cycle, otherwise)))
        }
        ExprKind::Let(def, body) => {
            let body = optimize(cx, t, cycle, body);
            optimize_def(cx, t, cycle, def, body)
        }
        ExprKind::LetRec(defs, body) => {
            let_rec(cx, t, cycle, defs, |t| optimize(cx, t, cycle, body))
        }
        ExprKind::LetDestruct {
            pattern,
            value,
            body,
        } => {
            let (name, destructors) = destruct(cx, t, pattern);
            let expr = optimize(cx, t, cycle, value);
            let body = optimize(cx, t, cycle, body);
            Expr::Let(
                Box::new(Def::Def(name, expr)),
                Box::new(wrap(destructors, body)),
            )
        }
        ExprKind::Case(subject, branches) => optimize_case(
            cx,
            t,
            subject,
            branches,
            |cx, t, b| optimize(cx, t, cycle, b),
            cycle,
        ),
        ExprKind::Accessor(field) => {
            t.field(field);
            Expr::Accessor(field.clone())
        }
        ExprKind::Access { record, field, .. } => {
            let record = optimize(cx, t, cycle, record);
            t.field(field);
            Expr::Access(Box::new(record), field.clone())
        }
        ExprKind::Update {
            record,
            fields: updates,
            ..
        } => {
            for field in updates.keys() {
                t.field(field);
            }
            let record = optimize(cx, t, cycle, record);
            let updates = (in_name_order(cx.compat, updates).into_iter())
                .map(|(field, update)| (field.clone(), optimize(cx, t, cycle, &update.value)))
                .collect();
            Expr::Update(Box::new(record), updates)
        }
        ExprKind::Record(fields) => {
            for field in fields.keys() {
                t.field(field);
            }
            let fields = (in_name_order(cx.compat, fields).into_iter())
                .map(|(field, value)| (field.clone(), optimize(cx, t, cycle, value)))
                .collect();
            Expr::Record(fields)
        }
        ExprKind::Unit => {
            t.kernel("Utils");
            Expr::Unit
        }
        ExprKind::Tuple(a, b, c) => {
            t.kernel("Utils");
            let a = optimize(cx, t, cycle, a);
            let b = optimize(cx, t, cycle, b);
            let c = c.as_ref().map(|c| Box::new(optimize(cx, t, cycle, c)));
            Expr::Tuple(Box::new(a), Box::new(b), c)
        }
        ExprKind::Shader(source, types) => Expr::Shader {
            source: shader_source(source).into_bytes(),
            attributes: types.attribute.keys().cloned().collect(),
            uniforms: types.uniform.keys().cloned().collect(),
        },
    }
}

/// `Shader.fromChars`: a shader's source, JavaScript-ready, carriage
/// returns dropped.
fn shader_source(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for c in source.chars() {
        match c {
            '\r' => {}
            '\n' => out.push_str("\\n"),
            '"' | '\'' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// `foldr Opt.Destruct body destructors`.
pub(super) fn wrap(destructors: Vec<Destructor>, body: Expr) -> Expr {
    (destructors.into_iter().rev()).fold(body, |body, d| Expr::Destruct(d, Box::new(body)))
}

/// A definition's parameters and body.
pub(super) fn parts(def: &CDef) -> (Vec<&canonical::Pattern>, &canonical::Expr) {
    match def {
        CDef::Def { params, body, .. } => (params.iter().collect(), body),
        CDef::TypedDef { params, body, .. } => (params.iter().map(|(p, _)| p).collect(), body),
    }
}

/// A recursive `let` of `defs`: one is a function that may loop, more
/// are each optimized as they are; then `body`.
fn let_rec(
    cx: &Context,
    t: &mut Tracker,
    cycle: &Cycle,
    defs: &[CDef],
    body: impl FnOnce(&mut Tracker) -> Expr,
) -> Expr {
    if let [def] = defs {
        let def = potential_tail_call_def(cx, t, cycle, def);
        Expr::Let(Box::new(def), Box::new(body(t)))
    } else {
        let body = body(t);
        defs.iter()
            .fold(body, |body, def| optimize_def(cx, t, cycle, def, body))
    }
}

/// `optimizeDef`: `def`, then `body` in its scope.
fn optimize_def(cx: &Context, t: &mut Tracker, cycle: &Cycle, def: &CDef, body: Expr) -> Expr {
    let (params, expr) = parts(def);
    let expr = optimize(cx, t, cycle, expr);
    let value = if params.is_empty() {
        expr
    } else {
        let (names, destructors) = destruct_args(cx, t, params);
        Expr::Function(names, Box::new(wrap(destructors, expr)))
    };
    Expr::Let(
        Box::new(Def::Def(def.name().to_owned(), value)),
        Box::new(body),
    )
}

/// `destructArgs`: a name for each argument, and what to take from those
/// that are patterns.
pub(super) fn destruct_args<'p>(
    cx: &Context,
    t: &mut Tracker,
    args: impl IntoIterator<Item = &'p canonical::Pattern>,
) -> (Vec<String>, Vec<Destructor>) {
    let mut names = Vec::new();
    let mut destructors = Vec::new();
    for arg in args {
        let (name, mut more) = destruct(cx, t, arg);
        names.push(name);
        destructors.append(&mut more);
    }
    (names, destructors)
}

/// `destruct`: the name a pattern binds its value to, and what to take
/// from it.
fn destruct(
    cx: &Context,
    t: &mut Tracker,
    pattern: &canonical::Pattern,
) -> (String, Vec<Destructor>) {
    match &pattern.kind {
        PatternKind::Var(name) => (name.clone(), Vec::new()),
        PatternKind::Alias(sub, name) => {
            let mut rev = Vec::new();
            destruct_help(cx, t, Path::Root(name.clone()), sub, &mut rev);
            (name.clone(), rev)
        }
        _ => {
            let name = t.generate();
            let mut rev = Vec::new();
            destruct_help(cx, t, Path::Root(name.clone()), pattern, &mut rev);
            (name, rev)
        }
    }
}

/// `destructHelp`, pushing onto `rev`: upstream keeps that list newest
/// first and reverses it once done, so here it is pushed in order.
fn destruct_help(
    cx: &Context,
    t: &mut Tracker,
    path: Path,
    pattern: &canonical::Pattern,
    rev: &mut Vec<Destructor>,
) {
    let at = |n: usize, path: &Path| Path::Index(n, Box::new(path.clone()));
    nomi_base::grow(|| match &pattern.kind {
        PatternKind::Anything
        | PatternKind::Unit
        | PatternKind::Chr(_)
        | PatternKind::Str(_)
        | PatternKind::Int(_)
        | PatternKind::Bool(..) => {}
        PatternKind::Var(name) => rev.push(Destructor {
            name: name.clone(),
            path,
        }),
        PatternKind::Record(fields) => {
            for field in fields {
                t.field(field);
            }
            // 0.19.1's parser keeps the fields last first. Upstream puts
            // them before what it has taken, newest first, so its first
            // field comes out last.
            let written: Vec<&String> = if cx.compat == Compat::V0_19_1 {
                fields.iter().rev().collect()
            } else {
                fields.iter().collect()
            };
            for name in written.into_iter().rev() {
                rev.push(Destructor {
                    name: name.clone(),
                    path: Path::Field(name.clone(), Box::new(path.clone())),
                });
            }
        }
        PatternKind::Alias(sub, name) => {
            rev.push(Destructor {
                name: name.clone(),
                path,
            });
            destruct_help(cx, t, Path::Root(name.clone()), sub, rev);
        }
        PatternKind::Tuple(a, b, None) => destruct_two(cx, t, path, a, b, rev),
        PatternKind::Tuple(a, b, Some(c)) => {
            let root = rooted(t, path, rev);
            destruct_help(cx, t, at(0, &root), a, rev);
            destruct_help(cx, t, at(1, &root), b, rev);
            destruct_help(cx, t, at(2, &root), c, rev);
        }
        PatternKind::List(items) => destruct_list(cx, t, path, items, rev),
        PatternKind::Cons(head, tail) => destruct_two(cx, t, path, head, tail, rev),
        PatternKind::Ctor { union, args, .. } => {
            if let [arg] = args.as_slice() {
                let path = match union.opts {
                    CtorOpts::Unbox => Path::Unbox(Box::new(path)),
                    CtorOpts::Normal | CtorOpts::Enum => at(0, &path),
                };
                destruct_help(cx, t, path, &arg.arg, rev);
            } else {
                let root = rooted(t, path, rev);
                for arg in args {
                    destruct_help(cx, t, at(arg.index, &root), &arg.arg, rev);
                }
            }
        }
    });
}

/// `destructTwo`.
fn destruct_two(
    cx: &Context,
    t: &mut Tracker,
    path: Path,
    a: &canonical::Pattern,
    b: &canonical::Pattern,
    rev: &mut Vec<Destructor>,
) {
    let root = rooted(t, path, rev);
    destruct_help(cx, t, Path::Index(0, Box::new(root.clone())), a, rev);
    destruct_help(cx, t, Path::Index(1, Box::new(root)), b, rev);
}

/// `destructTwo` of a list pattern's first item and the list of the rest,
/// down to the empty one, which takes nothing.
fn destruct_list(
    cx: &Context,
    t: &mut Tracker,
    mut path: Path,
    items: &[canonical::Pattern],
    rev: &mut Vec<Destructor>,
) {
    for item in items {
        let root = rooted(t, path, rev);
        destruct_help(cx, t, Path::Index(0, Box::new(root.clone())), item, rev);
        path = Path::Index(1, Box::new(root));
    }
}

/// `path` where it is a root; else a generated name, bound to it first.
fn rooted(t: &mut Tracker, path: Path, rev: &mut Vec<Destructor>) -> Path {
    if let Path::Root(_) = path {
        return path;
    }
    let name = t.generate();
    rev.push(Destructor {
        name: name.clone(),
        path,
    });
    Path::Root(name)
}

/// `optimizePotentialTailCallDef`.
fn potential_tail_call_def(cx: &Context, t: &mut Tracker, cycle: &Cycle, def: &CDef) -> Def {
    let (params, body) = parts(def);
    potential_tail_call(cx, t, cycle, def.name(), params, body)
}

/// `optimizePotentialTailCall`: a function, as a loop if it calls itself
/// in tail position.
pub(super) fn potential_tail_call(
    cx: &Context,
    t: &mut Tracker,
    cycle: &Cycle,
    name: &str,
    params: Vec<&canonical::Pattern>,
    body: &canonical::Expr,
) -> Def {
    let (names, destructors) = destruct_args(cx, t, params);
    let body = optimize_tail(cx, t, cycle, name, &names, body);
    if has_tail_call(&body) {
        Def::TailDef {
            name: name.to_owned(),
            params: names,
            body: wrap(destructors, body),
        }
    } else {
        Def::Def(
            name.to_owned(),
            Expr::Function(names, Box::new(wrap(destructors, body))),
        )
    }
}

/// `optimizeTail`: `expr` in tail position of the function `root`.
fn optimize_tail(
    cx: &Context,
    t: &mut Tracker,
    cycle: &Cycle,
    root: &str,
    args: &[String],
    expr: &canonical::Expr,
) -> Expr {
    nomi_base::grow(|| match &expr.kind {
        ExprKind::Call(func, call_args) => {
            let oargs: Vec<Expr> = call_args
                .iter()
                .map(|a| optimize(cx, t, cycle, a))
                .collect();
            let matching = match &func.kind {
                ExprKind::VarLocal(name) | ExprKind::VarTopLevel(_, name) => name == root,
                _ => false,
            };
            if matching && args.len() == oargs.len() {
                Expr::TailCall(root.to_owned(), args.iter().cloned().zip(oargs).collect())
            } else {
                let func = optimize(cx, t, cycle, func);
                Expr::Call(Box::new(func), oargs)
            }
        }
        ExprKind::If(branches, otherwise) => {
            let branches = (branches.iter())
                .map(|(c, b)| {
                    let c = optimize(cx, t, cycle, c);
                    (c, optimize_tail(cx, t, cycle, root, args, b))
                })
                .collect();
            Expr::If(
                branches,
                Box::new(optimize_tail(cx, t, cycle, root, args, otherwise)),
            )
        }
        ExprKind::Let(def, body) => {
            let body = optimize_tail(cx, t, cycle, root, args, body);
            optimize_def(cx, t, cycle, def, body)
        }
        ExprKind::LetRec(defs, body) => let_rec(cx, t, cycle, defs, |t| {
            optimize_tail(cx, t, cycle, root, args, body)
        }),
        ExprKind::LetDestruct {
            pattern,
            value,
            body,
        } => {
            let (name, destructors) = destruct(cx, t, pattern);
            let value = optimize(cx, t, cycle, value);
            let body = optimize_tail(cx, t, cycle, root, args, body);
            Expr::Let(
                Box::new(Def::Def(name, value)),
                Box::new(wrap(destructors, body)),
            )
        }
        ExprKind::Case(subject, branches) => optimize_case(
            cx,
            t,
            subject,
            branches,
            |cx, t, b| optimize_tail(cx, t, cycle, root, args, b),
            cycle,
        ),
        ExprKind::VarLocal(_)
        | ExprKind::VarTopLevel(..)
        | ExprKind::VarKernel { .. }
        | ExprKind::VarForeign { .. }
        | ExprKind::VarCtor { .. }
        | ExprKind::VarDebug { .. }
        | ExprKind::VarOperator(_)
        | ExprKind::Chr(_)
        | ExprKind::Str(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::List(_)
        | ExprKind::Negate(_)
        | ExprKind::Binop { .. }
        | ExprKind::Lambda(..)
        | ExprKind::Accessor(_)
        | ExprKind::Access { .. }
        | ExprKind::Update { .. }
        | ExprKind::Record(_)
        | ExprKind::Unit
        | ExprKind::Tuple(..)
        | ExprKind::Shader(..) => optimize(cx, t, cycle, expr),
    })
}

/// A `case`: a name for it, its subject, then each branch's destructors
/// and body, `body` optimizing those. A subject that isn't a variable is
/// bound to the generated name first.
fn optimize_case(
    cx: &Context,
    t: &mut Tracker,
    subject: &canonical::Expr,
    branches: &[(canonical::Pattern, canonical::Expr)],
    mut body: impl FnMut(&Context, &mut Tracker, &canonical::Expr) -> Expr,
    cycle: &Cycle,
) -> Expr {
    let temp = t.generate();
    let subject = optimize(cx, t, cycle, subject);
    let root = match &subject {
        Expr::VarLocal(root) => root.clone(),
        _ => temp.clone(),
    };
    let branches: Vec<(&canonical::Pattern, Expr)> = (branches.iter())
        .map(|(pattern, branch)| {
            let mut destructors = Vec::new();
            destruct_help(cx, t, Path::Root(root.clone()), pattern, &mut destructors);
            let branch = body(cx, t, branch);
            (pattern, wrap(destructors, branch))
        })
        .collect();
    match subject {
        Expr::VarLocal(_) => case::optimize(cx.compat, temp, root, branches),
        _ => Expr::Let(
            Box::new(Def::Def(temp.clone(), subject)),
            Box::new(case::optimize(cx.compat, temp.clone(), temp, branches)),
        ),
    }
}

/// `hasTailCall`.
fn has_tail_call(expr: &Expr) -> bool {
    nomi_base::grow(|| match expr {
        Expr::TailCall(..) => true,
        Expr::If(branches, otherwise) => {
            has_tail_call(otherwise) || branches.iter().any(|(_, b)| has_tail_call(b))
        }
        Expr::Let(_, body) | Expr::Destruct(_, body) => has_tail_call(body),
        Expr::Case { decider, jumps, .. } => {
            decider_has_tail_call(decider) || jumps.iter().any(|(_, e)| has_tail_call(e))
        }
        Expr::Bool(_)
        | Expr::Chr(_)
        | Expr::Str(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::VarLocal(_)
        | Expr::VarGlobal(_)
        | Expr::VarEnum(..)
        | Expr::VarBox(_)
        | Expr::VarCycle(..)
        | Expr::VarDebug { .. }
        | Expr::VarKernel { .. }
        | Expr::List(_)
        | Expr::Function(..)
        | Expr::Call(..)
        | Expr::Accessor(_)
        | Expr::Access(..)
        | Expr::Update(..)
        | Expr::Record(_)
        | Expr::Unit
        | Expr::Tuple(..)
        | Expr::Shader { .. } => false,
    })
}

/// `hasTailCall` of a decision's inlined branches.
fn decider_has_tail_call(decider: &Decider<Choice>) -> bool {
    nomi_base::grow(|| match decider {
        Decider::Leaf(Choice::Inline(expr)) => has_tail_call(expr),
        Decider::Leaf(Choice::Jump(_)) => false,
        Decider::Chain {
            success, failure, ..
        } => decider_has_tail_call(success) || decider_has_tail_call(failure),
        Decider::FanOut {
            tests, fallback, ..
        } => decider_has_tail_call(fallback) || tests.iter().any(|(_, d)| decider_has_tail_call(d)),
    })
}
