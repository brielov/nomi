//! Exhaustiveness and redundancy (`Nitpick/PatternMatches.hs`), by Luc
//! Maranget's "Warnings for Pattern Matching": every `case`, function
//! argument and destructuring must cover every value of its type, and no
//! branch of a `case` may be covered by the ones before it.
//!
//! Upstream gathers errors by prepending them through nested folds, so
//! their order follows its traversal exactly. Here the list is a vector
//! kept reversed, its end the list's head, and the traversal visits in
//! upstream's nesting order: what a fold applies innermost first.

use std::collections::{BTreeMap, VecDeque};
use std::sync::LazyLock;

use nomi_base::{Compat, TextRange};

use crate::canonical::{
    self, Ctor, CtorOpts, Decl, Def, Expr, ExprKind, Home, PatternKind, Type, Union,
};
use crate::types::in_name_order;

/// `Pattern`: a pattern as the checks see it, borrowing its union, name
/// and literal from the canonical pattern it simplifies.
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern<'u> {
    Anything,
    Literal(Literal<'u>),
    Ctor(&'u Union, &'u str, Vec<Pattern<'u>>),
}

/// `Literal`, as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Literal<'u> {
    Chr(&'u str),
    Str(&'u str),
    Int(i64),
}

/// `Error`.
#[derive(Clone, Debug, PartialEq)]
pub enum Error<'u> {
    /// Patterns at `region` that miss the `missing` ones.
    Incomplete {
        region: TextRange,
        context: Context,
        missing: Vec<Pattern<'u>>,
    },
    /// A `case`'s pattern the ones before it cover, by its place from 1.
    Redundant {
        case: TextRange,
        pattern: TextRange,
        index: usize,
    },
}

/// `Context`: what a pattern belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    BadArg,
    BadDestruct,
    BadCase,
}

pub(super) const UNIT: &str = "#0";
pub(super) const PAIR: &str = "#2";
pub(super) const TRIPLE: &str = "#3";
pub(super) const CONS: &str = "::";
pub(super) const NIL: &str = "[]";

/// A union of the type variables `vars`, its constructors `ctors` by
/// name and arguments, indexed in order.
fn union(vars: &[&str], ctors: Vec<(&str, Vec<Type>)>) -> Union {
    let count = ctors.len();
    Union {
        vars: vars.iter().map(|v| (*v).to_owned()).collect(),
        ctors: (ctors.into_iter().enumerate())
            .map(|(index, (name, args))| Ctor {
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

static UNIT_UNION: LazyLock<Union> = LazyLock::new(|| union(&[], vec![(UNIT, vec![])]));

static PAIR_UNION: LazyLock<Union> =
    LazyLock::new(|| union(&["a", "b"], vec![(PAIR, vec![var("a"), var("b")])]));

static TRIPLE_UNION: LazyLock<Union> = LazyLock::new(|| {
    union(
        &["a", "b", "c"],
        vec![(TRIPLE, vec![var("a"), var("b"), var("c")])],
    )
});

static LIST_UNION: LazyLock<Union> = LazyLock::new(|| {
    let list = Type::Type {
        home: Home::core("List"),
        name: "List".to_owned(),
        args: vec![var("a")],
    };
    union(&["a"], vec![(NIL, vec![]), (CONS, vec![var("a"), list])])
});

/// `simplify`.
pub fn simplify(pattern: &canonical::Pattern) -> Pattern<'_> {
    match &pattern.kind {
        PatternKind::Anything | PatternKind::Var(_) | PatternKind::Record(_) => Pattern::Anything,
        PatternKind::Unit => Pattern::Ctor(&UNIT_UNION, UNIT, Vec::new()),
        PatternKind::Tuple(a, b, None) => {
            Pattern::Ctor(&PAIR_UNION, PAIR, vec![simplify(a), simplify(b)])
        }
        PatternKind::Tuple(a, b, Some(c)) => Pattern::Ctor(
            &TRIPLE_UNION,
            TRIPLE,
            vec![simplify(a), simplify(b), simplify(c)],
        ),
        PatternKind::Ctor {
            union, name, args, ..
        } => Pattern::Ctor(union, name, args.iter().map(|a| simplify(&a.arg)).collect()),
        PatternKind::List(entries) => (entries.iter().rev())
            .fold(Pattern::Ctor(&LIST_UNION, NIL, Vec::new()), |tail, head| {
                cons(head, tail)
            }),
        PatternKind::Cons(head, tail) => cons(head, simplify(tail)),
        PatternKind::Alias(sub, _) => simplify(sub),
        PatternKind::Int(n) => Pattern::Literal(Literal::Int(*n)),
        PatternKind::Str(s) => Pattern::Literal(Literal::Str(s)),
        PatternKind::Chr(c) => Pattern::Literal(Literal::Chr(c)),
        PatternKind::Bool(union, b) => {
            Pattern::Ctor(union, if *b { "True" } else { "False" }, Vec::new())
        }
    }
}

fn cons<'u>(head: &'u canonical::Pattern, tail: Pattern<'u>) -> Pattern<'u> {
    Pattern::Ctor(&LIST_UNION, CONS, vec![simplify(head), tail])
}

/// `check`: the module's pattern errors, in upstream's order.
pub fn check(compat: Compat, decls: &[Decl]) -> Vec<Error<'_>> {
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

struct Checker<'u> {
    compat: Compat,
    /// Upstream's list, reversed.
    errors: Vec<Error<'u>>,
}

impl<'u> Checker<'u> {
    /// `checkDef`.
    fn def(&mut self, def: &'u Def) {
        match def {
            Def::Def { params, body, .. } => self.function(params.iter(), body),
            Def::TypedDef { params, body, .. } => {
                self.function(params.iter().map(|(p, _)| p), body);
            }
        }
    }

    /// A function's body, then its parameters, the last first.
    fn function(
        &mut self,
        params: impl DoubleEndedIterator<Item = &'u canonical::Pattern>,
        body: &'u Expr,
    ) {
        self.expr(body);
        for p in params.rev() {
            self.patterns(p.region, Context::BadArg, [p]);
        }
    }

    /// `checkExpr`.
    fn expr(&mut self, expr: &'u Expr) {
        nomi_base::grow(|| self.expr_(expr));
    }

    fn expr_(&mut self, expr: &'u Expr) {
        match &expr.kind {
            ExprKind::List(entries) => entries.iter().rev().for_each(|e| self.expr(e)),
            ExprKind::Negate(e) => self.expr(e),
            ExprKind::Binop { left, right, .. } => {
                self.expr(right);
                self.expr(left);
            }
            ExprKind::Lambda(params, body) => self.function(params.iter(), body),
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
            ExprKind::LetDestruct {
                pattern,
                value,
                body,
            } => {
                self.expr(body);
                self.expr(value);
                self.patterns(pattern.region, Context::BadDestruct, [pattern]);
            }
            ExprKind::Case(scrutinee, branches) => {
                for (_, body) in branches.iter().rev() {
                    self.expr(body);
                }
                self.patterns(
                    expr.region,
                    Context::BadCase,
                    branches.iter().map(|(p, _)| p),
                );
                self.expr(scrutinee);
            }
            ExprKind::Access { record, .. } => self.expr(record),
            ExprKind::Update { record, fields, .. } => {
                for (_, field) in in_name_order(self.compat, fields).into_iter().rev() {
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
            | ExprKind::VarKernel { .. }
            | ExprKind::VarForeign { .. }
            | ExprKind::VarCtor { .. }
            | ExprKind::VarDebug { .. }
            | ExprKind::VarOperator(_)
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
    fn patterns(
        &mut self,
        region: TextRange,
        context: Context,
        patterns: impl IntoIterator<Item = &'u canonical::Pattern>,
    ) {
        match non_redundant_rows(region, patterns) {
            Err(error) => self.errors.push(error),
            Ok(matrix) => {
                let bad = is_exhaustive(&matrix, 1);
                if !bad.is_empty() {
                    let missing = bad
                        .into_iter()
                        .map(|row| row.into_iter().next().expect("a column"))
                        .collect();
                    self.errors.push(Error::Incomplete {
                        region,
                        context,
                        missing,
                    });
                }
            }
        }
    }
}

/// `collectCtors`: each constructor at the head of a row, with its union.
fn collect_ctors<'u>(matrix: &[Vec<Pattern<'u>>]) -> BTreeMap<&'u str, &'u Union> {
    let mut ctors = BTreeMap::new();
    for row in matrix {
        if let Some(Pattern::Ctor(union, name, _)) = row.first() {
            ctors.insert(*name, *union);
        }
    }
    ctors
}

/// `isExhaustive`: the rows of `n` columns the matrix misses.
fn is_exhaustive<'u>(matrix: &[Vec<Pattern<'u>>], n: usize) -> Vec<Vec<Pattern<'u>>> {
    nomi_base::grow(|| {
        if matrix.is_empty() {
            return vec![vec![Pattern::Anything; n]];
        }
        if n == 0 {
            return Vec::new();
        }
        let ctors = collect_ctors(matrix);
        let Some(&alts) = ctors.values().next() else {
            let rest: Vec<Vec<Pattern>> = matrix
                .iter()
                .filter_map(|r| specialize_anything(r))
                .collect();
            return prepend(&Pattern::Anything, is_exhaustive(&rest, n - 1));
        };
        if ctors.len() < alts.count {
            let rest: Vec<Vec<Pattern>> = matrix
                .iter()
                .filter_map(|r| specialize_anything(r))
                .collect();
            let tails = is_exhaustive(&rest, n - 1);
            // `(:) <$> missing <*> tails`.
            return (alts.ctors.iter())
                .filter(|c| !ctors.contains_key(c.name.as_str()))
                .flat_map(|c| {
                    let missing =
                        Pattern::Ctor(alts, &c.name, vec![Pattern::Anything; c.args.len()]);
                    prepend(&missing, tails.clone())
                })
                .collect();
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
                let mut row = vec![Pattern::Ctor(alts, &ctor.name, args)];
                row.extend(missing);
                rows.push(row);
            }
        }
        rows
    })
}

fn prepend<'u>(head: &Pattern<'u>, rows: Vec<Vec<Pattern<'u>>>) -> Vec<Vec<Pattern<'u>>> {
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
fn non_redundant_rows<'u>(
    region: TextRange,
    patterns: impl IntoIterator<Item = &'u canonical::Pattern>,
) -> Result<Vec<Vec<Pattern<'u>>>, Error<'u>> {
    // Upstream conses each new row onto the checked ones, so the newest
    // comes first.
    let mut checked: VecDeque<Vec<Pattern>> = VecDeque::new();
    for pattern in patterns {
        let row = vec![simplify(pattern)];
        if is_useful(checked.make_contiguous(), &row) {
            checked.push_front(row);
        } else {
            return Err(Error::Redundant {
                case: region,
                pattern: pattern.region,
                index: checked.len() + 1,
            });
        }
    }
    Ok(checked.into())
}

/// `isUseful`: whether `vector` matches a value no row does.
fn is_useful(matrix: &[Vec<Pattern>], vector: &[Pattern]) -> bool {
    nomi_base::grow(|| {
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
                    .filter_map(|r| specialize_literal(*literal, r))
                    .collect();
                is_useful(&specialized, patterns)
            }
        }
    })
}

/// `isComplete`: the union's constructors when every one heads a row.
fn is_complete<'u>(matrix: &[Vec<Pattern<'u>>]) -> Option<&'u [Ctor]> {
    let ctors = collect_ctors(matrix);
    let alts = *ctors.values().next()?;
    (ctors.len() == alts.count).then_some(alts.ctors.as_slice())
}

/// `specializeRowByCtor`.
fn specialize_ctor<'u>(name: &str, arity: usize, row: &[Pattern<'u>]) -> Option<Vec<Pattern<'u>>> {
    let (first, rest) = row
        .split_first()
        .expect("Compiler error! Empty matrices should not get specialized.");
    match first {
        Pattern::Ctor(_, n, args) => {
            (*n == name).then(|| args.iter().chain(rest).cloned().collect())
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
fn specialize_literal<'u>(literal: Literal<'_>, row: &[Pattern<'u>]) -> Option<Vec<Pattern<'u>>> {
    let (first, rest) = row
        .split_first()
        .expect("Compiler error! Empty matrices should not get specialized.");
    match first {
        Pattern::Literal(l) => (*l == literal).then(|| rest.to_vec()),
        Pattern::Anything => Some(rest.to_vec()),
        Pattern::Ctor(..) => panic!(
            "Compiler bug! After type checking, constructors and literals should never align in pattern match exhaustiveness checks."
        ),
    }
}

/// `specializeRowByAnything`.
fn specialize_anything<'u>(row: &[Pattern<'u>]) -> Option<Vec<Pattern<'u>>> {
    match row.split_first()? {
        (Pattern::Anything, rest) => Some(rest.to_vec()),
        (Pattern::Literal(_) | Pattern::Ctor(..), _) => None,
    }
}
