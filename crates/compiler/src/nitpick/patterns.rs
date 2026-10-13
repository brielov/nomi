//! Exhaustiveness and redundancy (`Nitpick/PatternMatches.hs`), by Luc
//! Maranget's "Warnings for Pattern Matching": every `case`, function
//! argument and destructuring must cover every value of its type, and no
//! branch of a `case` may be covered by the ones before it.
//!
//! Upstream gathers errors by prepending them through nested folds, so
//! their order follows its traversal exactly. Here the list is a vector
//! kept reversed, its end the list's head, and the traversal visits in
//! upstream's nesting order: what a fold applies innermost first.

use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange};

use crate::canonical::{
    self, Ctor, CtorOpts, Decl, Def, Expr, ExprKind, Home, PatternKind, Type, Union,
};
use crate::types::in_name_order;

/// `Pattern`: a pattern as the checks see it.
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Anything,
    Literal(Literal),
    Ctor(Union, String, Vec<Pattern>),
}

/// `Literal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Literal {
    Chr(String),
    Str(String),
    Int(i64),
}

/// `Error`.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The region checked, what it is, and patterns it misses.
    Incomplete(TextRange, Context, Vec<Pattern>),
    /// The `case`'s region, the redundant pattern's, and its place from 1.
    Redundant(TextRange, TextRange, usize),
}

/// `Context`: what a pattern belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    BadArg,
    BadDestruct,
    BadCase,
}

const UNIT: &str = "#0";
const PAIR: &str = "#2";
const TRIPLE: &str = "#3";
const CONS: &str = "::";
const NIL: &str = "[]";

fn union(vars: &[&str], ctors: Vec<(&str, usize, Vec<Type>)>) -> Union {
    let count = ctors.len();
    Union {
        vars: vars.iter().map(|v| (*v).to_owned()).collect(),
        ctors: (ctors.into_iter())
            .map(|(name, index, args)| Ctor {
                name: name.to_owned(),
                index,
                args,
            })
            .collect(),
        count,
        opts: CtorOpts::Normal,
    }
}

fn var(name: &str) -> Type {
    Type::Var(name.to_owned())
}

fn unit() -> Union {
    union(&[], vec![(UNIT, 0, vec![])])
}

fn pair() -> Union {
    union(&["a", "b"], vec![(PAIR, 0, vec![var("a"), var("b")])])
}

fn triple() -> Union {
    union(
        &["a", "b", "c"],
        vec![(TRIPLE, 0, vec![var("a"), var("b"), var("c")])],
    )
}

fn list() -> Union {
    let list = Type::Type(
        Home {
            package: "elm/core".into(),
            module: "List".into(),
        },
        "List".to_owned(),
        vec![var("a")],
    );
    union(
        &["a"],
        vec![(NIL, 0, vec![]), (CONS, 1, vec![var("a"), list])],
    )
}

/// `simplify`.
pub fn simplify(pattern: &canonical::Pattern) -> Pattern {
    match &pattern.kind {
        PatternKind::Anything | PatternKind::Var(_) | PatternKind::Record(_) => Pattern::Anything,
        PatternKind::Unit => Pattern::Ctor(unit(), UNIT.to_owned(), Vec::new()),
        PatternKind::Tuple(a, b, None) => {
            Pattern::Ctor(pair(), PAIR.to_owned(), vec![simplify(a), simplify(b)])
        }
        PatternKind::Tuple(a, b, Some(c)) => Pattern::Ctor(
            triple(),
            TRIPLE.to_owned(),
            vec![simplify(a), simplify(b), simplify(c)],
        ),
        PatternKind::Ctor {
            union, name, args, ..
        } => Pattern::Ctor(
            union.clone(),
            name.clone(),
            args.iter().map(|a| simplify(&a.arg)).collect(),
        ),
        PatternKind::List(entries) => (entries.iter().rev()).fold(
            Pattern::Ctor(list(), NIL.to_owned(), Vec::new()),
            |tail, head| cons(head, tail),
        ),
        PatternKind::Cons(head, tail) => cons(head, simplify(tail)),
        PatternKind::Alias(sub, _) => simplify(sub),
        PatternKind::Int(n) => Pattern::Literal(Literal::Int(*n)),
        PatternKind::Str(s) => Pattern::Literal(Literal::Str(s.clone())),
        PatternKind::Chr(c) => Pattern::Literal(Literal::Chr(c.clone())),
        PatternKind::Bool(union, b) => Pattern::Ctor(
            union.clone(),
            if *b { "True" } else { "False" }.to_owned(),
            Vec::new(),
        ),
    }
}

fn cons(head: &canonical::Pattern, tail: Pattern) -> Pattern {
    Pattern::Ctor(list(), CONS.to_owned(), vec![simplify(head), tail])
}

/// `check`: the module's pattern errors, in upstream's order.
pub fn check(compat: Compat, decls: &[Decl]) -> Vec<Error> {
    let mut checker = Checker {
        compat,
        errors: Vec::new(),
    };
    for decl in decls.iter().rev() {
        match decl {
            Decl::Declare(def) => checker.def(def),
            Decl::DeclareRec(def, defs) => {
                for d in defs.iter().rev() {
                    checker.def(d);
                }
                checker.def(def);
            }
        }
    }
    let mut errors = checker.errors;
    errors.reverse();
    errors
}

struct Checker {
    compat: Compat,
    /// Upstream's list, reversed.
    errors: Vec<Error>,
}

impl Checker {
    /// `checkDef`.
    fn def(&mut self, def: &Def) {
        match def {
            Def::Def { params, body, .. } => {
                self.expr(body);
                for p in params.iter().rev() {
                    self.patterns(p.region, Context::BadArg, std::slice::from_ref(p));
                }
            }
            Def::TypedDef { params, body, .. } => {
                self.expr(body);
                for (p, _) in params.iter().rev() {
                    self.patterns(p.region, Context::BadArg, std::slice::from_ref(p));
                }
            }
        }
    }

    /// `checkExpr`.
    fn expr(&mut self, expr: &Expr) {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.expr_(expr));
    }

    fn expr_(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::List(entries) => entries.iter().rev().for_each(|e| self.expr(e)),
            ExprKind::Negate(e) => self.expr(e),
            ExprKind::Binop(_, _, _, _, left, right) => {
                self.expr(right);
                self.expr(left);
            }
            ExprKind::Lambda(params, body) => {
                self.expr(body);
                for p in params.iter().rev() {
                    self.patterns(p.region, Context::BadArg, std::slice::from_ref(p));
                }
            }
            ExprKind::Call(function, args) => {
                args.iter().rev().for_each(|a| self.expr(a));
                self.expr(function);
            }
            ExprKind::If(branches, otherwise) => {
                self.expr(otherwise);
                for (condition, branch) in branches.iter().rev() {
                    self.expr(branch);
                    self.expr(condition);
                }
            }
            ExprKind::Let(def, body) => {
                self.expr(body);
                self.def(def);
            }
            ExprKind::LetRec(defs, body) => {
                self.expr(body);
                defs.iter().rev().for_each(|d| self.def(d));
            }
            ExprKind::LetDestruct(pattern, value, body) => {
                self.expr(body);
                self.expr(value);
                self.patterns(
                    pattern.region,
                    Context::BadDestruct,
                    std::slice::from_ref(pattern),
                );
            }
            ExprKind::Case(scrutinee, branches) => {
                for (_, body) in branches.iter().rev() {
                    self.expr(body);
                }
                let patterns: Vec<canonical::Pattern> =
                    branches.iter().map(|(p, _)| p.clone()).collect();
                self.patterns(expr.region, Context::BadCase, &patterns);
                self.expr(scrutinee);
            }
            ExprKind::Access(record, _, _) => self.expr(record),
            ExprKind::Update(_, record, fields) => {
                let ordered: Vec<&canonical::FieldUpdate> = in_name_order(self.compat, fields)
                    .into_iter()
                    .map(|(_, f)| f)
                    .collect();
                for field in ordered.into_iter().rev() {
                    self.expr(&field.value);
                }
                self.expr(record);
            }
            ExprKind::Record(fields) => {
                for (_, e) in in_name_order(self.compat, fields).into_iter().rev() {
                    self.expr(e);
                }
            }
            ExprKind::Tuple(a, b, c) => {
                if let Some(c) = c {
                    self.expr(c);
                }
                self.expr(b);
                self.expr(a);
            }
            ExprKind::VarLocal(_)
            | ExprKind::VarTopLevel(..)
            | ExprKind::VarKernel(..)
            | ExprKind::VarForeign(..)
            | ExprKind::VarCtor(..)
            | ExprKind::VarDebug(..)
            | ExprKind::VarOperator(..)
            | ExprKind::Chr(_)
            | ExprKind::Str(_)
            | ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Accessor(_)
            | ExprKind::Unit
            | ExprKind::Shader(..) => {}
        }
    }

    /// `checkPatterns`.
    fn patterns(&mut self, region: TextRange, context: Context, patterns: &[canonical::Pattern]) {
        match non_redundant_rows(region, patterns) {
            Err(error) => self.errors.push(error),
            Ok(matrix) => {
                let bad = is_exhaustive(&matrix, 1);
                if !bad.is_empty() {
                    let missing = bad
                        .into_iter()
                        .map(|row| row.into_iter().next().expect("a column"))
                        .collect();
                    self.errors
                        .push(Error::Incomplete(region, context, missing));
                }
            }
        }
    }
}

/// `collectCtors`: each constructor at the head of a row, with its union.
fn collect_ctors(matrix: &[Vec<Pattern>]) -> BTreeMap<&str, &Union> {
    let mut ctors = BTreeMap::new();
    for row in matrix {
        if let Some(Pattern::Ctor(union, name, _)) = row.first() {
            ctors.insert(name.as_str(), union);
        }
    }
    ctors
}

/// `isExhaustive`: the rows of `n` columns the matrix misses.
fn is_exhaustive(matrix: &[Vec<Pattern>], n: usize) -> Vec<Vec<Pattern>> {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        if matrix.is_empty() {
            return vec![vec![Pattern::Anything; n]];
        }
        if n == 0 {
            return Vec::new();
        }
        let ctors = collect_ctors(matrix);
        if ctors.is_empty() {
            let rest: Vec<Vec<Pattern>> = matrix
                .iter()
                .filter_map(|r| specialize_anything(r))
                .collect();
            return prepend(&Pattern::Anything, is_exhaustive(&rest, n - 1));
        }
        let alts = (*ctors.values().next().expect("a constructor")).clone();
        if ctors.len() < alts.count {
            let missing: Vec<Pattern> = (alts.ctors.iter())
                .filter(|c| !ctors.contains_key(c.name.as_str()))
                .map(|c| {
                    Pattern::Ctor(
                        alts.clone(),
                        c.name.clone(),
                        vec![Pattern::Anything; c.args.len()],
                    )
                })
                .collect();
            let rest: Vec<Vec<Pattern>> = matrix
                .iter()
                .filter_map(|r| specialize_anything(r))
                .collect();
            let tails = is_exhaustive(&rest, n - 1);
            // `(:) <$> missing <*> tails`.
            let mut rows = Vec::new();
            for m in &missing {
                for tail in &tails {
                    let mut row = vec![m.clone()];
                    row.extend(tail.iter().cloned());
                    rows.push(row);
                }
            }
            return rows;
        }
        let mut rows = Vec::new();
        for ctor in &alts.ctors {
            let arity = ctor.args.len();
            let specialized: Vec<Vec<Pattern>> = matrix
                .iter()
                .filter_map(|r| specialize_ctor(&ctor.name, arity, r))
                .collect();
            for missing in is_exhaustive(&specialized, arity + n - 1) {
                let mut missing = missing.into_iter();
                let args: Vec<Pattern> = missing.by_ref().take(arity).collect();
                let mut row = vec![Pattern::Ctor(alts.clone(), ctor.name.clone(), args)];
                row.extend(missing);
                rows.push(row);
            }
        }
        rows
    })
}

fn prepend(head: &Pattern, rows: Vec<Vec<Pattern>>) -> Vec<Vec<Pattern>> {
    (rows.into_iter())
        .map(|row| {
            let mut new = vec![head.clone()];
            new.extend(row);
            new
        })
        .collect()
}

/// `toNonRedundantRows`: each pattern a row, or the first one the rows
/// before it already cover.
fn non_redundant_rows(
    region: TextRange,
    patterns: &[canonical::Pattern],
) -> Result<Vec<Vec<Pattern>>, Error> {
    // Upstream conses each new row onto the checked ones, so the newest
    // comes first.
    let mut checked: std::collections::VecDeque<Vec<Pattern>> = std::collections::VecDeque::new();
    for pattern in patterns {
        let row = vec![simplify(pattern)];
        if is_useful(checked.make_contiguous(), &row) {
            checked.push_front(row);
        } else {
            return Err(Error::Redundant(region, pattern.region, checked.len() + 1));
        }
    }
    Ok(checked.into())
}

/// `isUseful`: whether `vector` matches a value no row does.
fn is_useful(matrix: &[Vec<Pattern>], vector: &[Pattern]) -> bool {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        if matrix.is_empty() {
            return true;
        }
        let Some((first, patterns)) = vector.split_first() else {
            return false;
        };
        match first {
            Pattern::Ctor(_, name, args) => {
                let specialized: Vec<Vec<Pattern>> = matrix
                    .iter()
                    .filter_map(|r| specialize_ctor(name, args.len(), r))
                    .collect();
                let mut next = args.clone();
                next.extend(patterns.iter().cloned());
                is_useful(&specialized, &next)
            }
            Pattern::Anything => match is_complete(matrix) {
                None => {
                    let rest: Vec<Vec<Pattern>> = matrix
                        .iter()
                        .filter_map(|r| specialize_anything(r))
                        .collect();
                    is_useful(&rest, patterns)
                }
                Some(alts) => alts.iter().any(|ctor| {
                    let arity = ctor.args.len();
                    let specialized: Vec<Vec<Pattern>> = matrix
                        .iter()
                        .filter_map(|r| specialize_ctor(&ctor.name, arity, r))
                        .collect();
                    let mut next = vec![Pattern::Anything; arity];
                    next.extend(patterns.iter().cloned());
                    is_useful(&specialized, &next)
                }),
            },
            Pattern::Literal(literal) => {
                let specialized: Vec<Vec<Pattern>> = matrix
                    .iter()
                    .filter_map(|r| specialize_literal(literal, r))
                    .collect();
                is_useful(&specialized, patterns)
            }
        }
    })
}

/// `isComplete`: the union's constructors when every one heads a row.
fn is_complete(matrix: &[Vec<Pattern>]) -> Option<Vec<Ctor>> {
    let ctors = collect_ctors(matrix);
    let alts = *ctors.values().next()?;
    (ctors.len() == alts.count).then(|| alts.ctors.clone())
}

/// `specializeRowByCtor`.
fn specialize_ctor(name: &str, arity: usize, row: &[Pattern]) -> Option<Vec<Pattern>> {
    let (first, rest) = row
        .split_first()
        .expect("Compiler error! Empty matrices should not get specialized.");
    match first {
        Pattern::Ctor(_, n, args) => {
            (n == name).then(|| args.iter().chain(rest).cloned().collect())
        }
        Pattern::Anything => {
            let mut new = vec![Pattern::Anything; arity];
            new.extend(rest.iter().cloned());
            Some(new)
        }
        Pattern::Literal(_) => panic!(
            "Compiler bug! After type checking, constructors and literals should never align in pattern match exhaustiveness checks."
        ),
    }
}

/// `specializeRowByLiteral`.
fn specialize_literal(literal: &Literal, row: &[Pattern]) -> Option<Vec<Pattern>> {
    let (first, rest) = row
        .split_first()
        .expect("Compiler error! Empty matrices should not get specialized.");
    match first {
        Pattern::Literal(l) => (l == literal).then(|| rest.to_vec()),
        Pattern::Anything => Some(rest.to_vec()),
        Pattern::Ctor(..) => panic!(
            "Compiler bug! After type checking, constructors and literals should never align in pattern match exhaustiveness checks."
        ),
    }
}

/// `specializeRowByAnything`.
fn specialize_anything(row: &[Pattern]) -> Option<Vec<Pattern>> {
    match row.split_first()? {
        (Pattern::Anything, rest) => Some(rest.to_vec()),
        _ => None,
    }
}
