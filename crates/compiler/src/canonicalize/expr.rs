//! Canonicalizing expressions (`Canonicalize/Expression.hs`): each
//! variable, constructor and operator resolved, every scope's bindings
//! checked, and the locals each scope uses counted (`FreeLocals`), which
//! the cycle checks of plan 5.12 read. A scope's uses count only when it
//! canonicalized cleanly, as upstream discards them otherwise.

use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange};

use super::binops;
use super::dups::{Dict, check_fields};
use super::env::{CtorInfo, Env, Var};
use super::error::{DuplicatePatternContext, Error, NameKind, PossibleNames, R, all, both, throw};
use super::foreign::{is_kernel_module, is_kernel_package};
use super::pattern::{self, Bindings};
use super::references::{Kind, Target};
use super::types;
use crate::canonical::{
    Annotation, CtorOpts, Def as CDef, Expr, ExprKind, FieldUpdate, Home, Pattern, Type,
};
use crate::hir::{self, Def as Def_, DefId, Expr as Expr_, ExprId, PatId};

/// How often a local is used: directly, or inside a function that may
/// run later (`Uses`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uses {
    pub direct: u32,
    pub delayed: u32,
}

/// `FreeLocals`.
pub type FreeLocals = BTreeMap<String, Uses>;

/// `Map.unionWith combineUses`, into `into`.
fn combine(into: &mut FreeLocals, from: FreeLocals) {
    for (name, uses) in from {
        let entry = into.entry(name).or_default();
        entry.direct += uses.direct;
        entry.delayed += uses.delayed;
    }
}

/// `delayUse`: every use as delayed.
fn delay(free: FreeLocals) -> FreeLocals {
    free.into_iter()
        .map(|(name, u)| {
            let delayed = Uses {
                direct: 0,
                delayed: u.direct + u.delayed,
            };
            (name, delayed)
        })
        .collect()
}

/// `verifyBindings`: what `work` makes in a scope of `bindings`, and the
/// locals it uses from outside the scope.
fn verify_bindings<T>(
    bindings: &BTreeMap<String, TextRange>,
    work: impl FnOnce(&mut FreeLocals) -> R<T>,
) -> R<(T, FreeLocals)> {
    let mut free = FreeLocals::new();
    let value = work(&mut free)?;
    free.retain(|name, _| !bindings.contains_key(name));
    Ok((value, free))
}

/// One binding of a `let`, as the cycle check sees it (`Binding`).
#[derive(Clone, Debug)]
pub enum LetBinding {
    Define(CDef),
    /// A name a destructuring binds.
    Edge {
        name: String,
        range: TextRange,
    },
    Destruct(Pattern, Expr),
}

/// A node of a `let`'s graph: the binding, its key, and the names it uses.
pub type LetNode = (LetBinding, String, Vec<String>);

fn range(module: &hir::Module, id: ExprId) -> TextRange {
    module
        .source_map
        .expr(id)
        .map(|p| p.range)
        .unwrap_or_default()
}

/// An integer literal's value, maybe negative, wrapping at 64 bits as
/// elm's parser does.
pub fn int_value(text: &str) -> i64 {
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (digits, radix) = match text.strip_prefix("0x") {
        Some(hex) => (hex, 16),
        None => (text, 10),
    };
    let value = (digits.chars())
        .filter_map(|c| c.to_digit(radix))
        .fold(0i64, |n, d| {
            n.wrapping_mul(i64::from(radix)).wrapping_add(i64::from(d))
        });
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

fn expr(region: TextRange, kind: ExprKind) -> Expr {
    Expr { region, kind }
}

/// `canonicalize`.
pub fn canonicalize(
    env: &Env<'_>,
    module: &hir::Module,
    id: ExprId,
    free: &mut FreeLocals,
) -> R<Expr> {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
        canonicalize_(env, module, id, free)
    })
}

fn each(
    env: &Env<'_>,
    module: &hir::Module,
    ids: &[ExprId],
    free: &mut FreeLocals,
) -> R<Vec<Expr>> {
    let results: Vec<R<Expr>> = ids
        .iter()
        .map(|&id| canonicalize(env, module, id, free))
        .collect();
    all(results)
}

fn canonicalize_(
    env: &Env<'_>,
    module: &hir::Module,
    id: ExprId,
    free: &mut FreeLocals,
) -> R<Expr> {
    let region = range(module, id);
    let kind = match &module.exprs[id] {
        // A module with syntax errors canonicalizes for the editor only.
        Expr_::Missing | Expr_::Unit => ExprKind::Unit,
        Expr_::Chr(text) => ExprKind::Chr(text.clone()),
        Expr_::Str(text) => ExprKind::Str(text.clone()),
        Expr_::Int(text) => ExprKind::Int(int_value(text)),
        Expr_::Float(text) => ExprKind::Float(text.clone()),
        Expr_::Accessor(field) => ExprKind::Accessor(field.clone()),
        Expr_::Shader(source) => ExprKind::Shader(source.clone(), super::shader::types(source)),
        Expr_::Var {
            qualifier,
            name,
            upper: false,
        } => match qualifier {
            None => find_var(env, region, name, free)?,
            Some(prefix) => find_var_qual(env, region, prefix, name)?,
        },
        Expr_::Var {
            qualifier, name, ..
        } => to_var_ctor(name, env.find_ctor(region, qualifier.as_deref(), name)?),
        Expr_::List(items) => ExprKind::List(each(env, module, items, free)?),
        Expr_::Op(op) => {
            let b = env.find_binop(region, op)?;
            ExprKind::VarOperator(
                op.clone(),
                b.home.clone(),
                b.function.clone(),
                b.annotation.clone(),
            )
        }
        Expr_::Negate(inner) => {
            ExprKind::Negate(Box::new(canonicalize(env, module, *inner, free)?))
        }
        Expr_::Binops { operands, ops } => {
            // Each operand with the operator after it, then the last, and
            // only then the chain arranged by precedence.
            let chain = all(operands
                .iter()
                .zip(ops)
                .enumerate()
                .map(|(i, (&operand, op))| {
                    let operand = canonicalize(env, module, operand, free);
                    let found = env.find_binop(op.range, &op.text).map(|b| {
                        let arranged = binops::Op {
                            index: i,
                            name: op.text.clone(),
                            associativity: b.associativity,
                            precedence: b.precedence,
                        };
                        let applied = (
                            op.text.clone(),
                            b.home.clone(),
                            b.function.clone(),
                            b.annotation.clone(),
                        );
                        (arranged, applied)
                    });
                    both(operand, found)
                })
                .collect::<Vec<_>>());
            let last = canonicalize(env, module, *operands.last().expect("an operand"), free);
            let (chain, last) = both(chain, last)?;
            let mut exprs: Vec<Option<Expr>> = Vec::with_capacity(chain.len() + 1);
            let mut applied = Vec::with_capacity(chain.len());
            let mut steps = Vec::with_capacity(chain.len());
            for (i, (operand, (op, info))) in chain.into_iter().enumerate() {
                exprs.push(Some(operand));
                applied.push(info);
                steps.push((binops::Tree::Operand(i), op));
            }
            exprs.push(Some(last));
            let tree = binops::arrange(steps, binops::Tree::Operand(ops.len()), region)?;
            // The chain's root keeps the chain's own region, as upstream
            // wraps the stepper's result in it; the nodes inside merge
            // their operands'.
            let mut root = binop_tree(tree, &mut exprs, &applied);
            root.region = region;
            return Ok(root);
        }
        Expr_::Lambda { params, body } => {
            let ((params, body), inner) = lambda(env, module, params, *body)?;
            combine(free, delay(inner));
            ExprKind::Lambda(params, Box::new(body))
        }
        Expr_::Call { function, args } => {
            let function = canonicalize(env, module, *function, free);
            let args = each(env, module, args, free);
            let (function, args) = both(function, args)?;
            // `A.merge func lastArg`: a parenthesized argument's region is
            // its inner expression's, so the call ends inside the paren.
            let end = args.last().map_or(region.end(), |a| a.region.end());
            let region = TextRange::new(function.region.start(), end);
            return Ok(expr(region, ExprKind::Call(Box::new(function), args)));
        }
        Expr_::If {
            branches,
            otherwise,
        } => {
            let branches = all(branches
                .iter()
                .map(|&(condition, then)| {
                    let condition = canonicalize(env, module, condition, free);
                    let then = canonicalize(env, module, then, free);
                    both(condition, then)
                })
                .collect::<Vec<_>>());
            let otherwise = canonicalize(env, module, *otherwise, free);
            let (branches, otherwise) = both(branches, otherwise)?;
            ExprKind::If(branches, Box::new(otherwise))
        }
        Expr_::Let { defs, body } => return let_(env, module, region, defs, *body, free),
        Expr_::Case {
            scrutinee,
            branches,
        } => {
            let scrutinee = canonicalize(env, module, *scrutinee, free);
            let branches = all(branches
                .iter()
                .map(|&(pattern, body)| {
                    let (branch, inner) = case_branch(env, module, pattern, body)?;
                    combine(free, inner);
                    Ok(branch)
                })
                .collect::<Vec<_>>());
            let (scrutinee, branches) = both(scrutinee, branches)?;
            ExprKind::Case(Box::new(scrutinee), branches)
        }
        Expr_::Access { record, field } => {
            let record = canonicalize(env, module, *record, free)?;
            ExprKind::Access(Box::new(record), field.range, field.text.clone())
        }
        Expr_::Update { record, fields } => {
            let found = find_var(env, record.range, &record.text, free)
                .map(|kind| expr(record.range, kind));
            let fields = (fields.iter())
                .map(|(name, value)| (name.text.clone(), name.range, (name.range, *value)))
                .collect();
            let fields = check_fields(env.compat, fields).and_then(|fields| {
                let names = super::sorted(env.compat, fields.keys());
                let values = all(names
                    .iter()
                    .map(|n| {
                        let (field_region, value) = fields[*n];
                        canonicalize(env, module, value, free).map(|value| {
                            let update = FieldUpdate {
                                region: field_region,
                                value,
                            };
                            ((*n).clone(), update)
                        })
                    })
                    .collect::<Vec<_>>());
                values.map(|v| v.into_iter().collect::<BTreeMap<_, _>>())
            });
            let (found, fields) = both(found, fields)?;
            ExprKind::Update(record.text.clone(), Box::new(found), fields)
        }
        Expr_::Record(fields) => {
            let fields = (fields.iter())
                .map(|(name, value)| (name.text.clone(), name.range, *value))
                .collect();
            let fields = check_fields(env.compat, fields)?;
            let names = super::sorted(env.compat, fields.keys());
            let values = all(names
                .iter()
                .map(|n| canonicalize(env, module, fields[*n], free).map(|v| ((*n).clone(), v)))
                .collect::<Vec<_>>())?;
            ExprKind::Record(values.into_iter().collect())
        }
        Expr_::Tuple(items) => {
            let a = canonicalize(env, module, items[0], free);
            let b = canonicalize(env, module, items[1], free);
            let rest = match &items[2..] {
                [] => Ok(None),
                [c] => canonicalize(env, module, *c, free).map(Some),
                _ => throw(Error::TupleLargerThanThree { range: region }),
            };
            let ((a, b), c) = both(both(a, b), rest)?;
            ExprKind::Tuple(Box::new(a), Box::new(b), c.map(Box::new))
        }
    };
    Ok(expr(region, kind))
}

/// An operator applied, as upstream's `toBinop` makes it: an operator
/// and the function it stands for.
type Applied = (String, Home, String, Annotation);

/// The expression an arranged chain stands for, each node's region from
/// its left operand's start to its right one's end (`A.merge`).
fn binop_tree(tree: binops::Tree, exprs: &mut [Option<Expr>], applied: &[Applied]) -> Expr {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || match tree {
        binops::Tree::Operand(i) => exprs[i].take().expect("each operand once"),
        binops::Tree::Binop { op, left, right } => {
            let left = binop_tree(*left, exprs, applied);
            let right = binop_tree(*right, exprs, applied);
            let region = TextRange::new(left.region.start(), right.region.end());
            let (name, home, function, annotation) = applied[op].clone();
            expr(
                region,
                ExprKind::Binop(
                    name,
                    home,
                    function,
                    annotation,
                    Box::new(left),
                    Box::new(right),
                ),
            )
        }
    })
}

/// `toVarCtor`: a constructor, with its type.
fn to_var_ctor(name: &str, ctor: &CtorInfo<'_>) -> ExprKind {
    match ctor {
        CtorInfo::Ctor {
            home,
            union_name,
            union,
            index,
            args,
        } => {
            let result = Type::Type(
                home.clone(),
                union_name.clone(),
                union.vars.iter().map(|v| Type::Var(v.clone())).collect(),
            );
            let ty = (args.iter().rev()).fold(result, |result, arg| {
                Type::Lambda(Box::new(arg.clone()), Box::new(result))
            });
            let annotation = Annotation {
                free: sorted_vars(&union.vars),
                ty,
            };
            ExprKind::VarCtor(
                union.opts,
                home.clone(),
                name.to_owned(),
                *index,
                annotation,
            )
        }
        CtorInfo::Record { home, vars, ty } => {
            let annotation = Annotation {
                free: sorted_vars(vars),
                ty: ty.clone(),
            };
            ExprKind::VarCtor(
                CtorOpts::Normal,
                home.clone(),
                name.to_owned(),
                0,
                annotation,
            )
        }
    }
}

/// A type's variables as a set, as `Map.fromList` keeps them.
fn sorted_vars(vars: &[String]) -> Vec<String> {
    let mut vars = vars.to_vec();
    vars.sort();
    vars.dedup();
    vars
}

/// A lambda: its parameters, bound once each, then its body.
fn lambda(
    env: &Env<'_>,
    module: &hir::Module,
    params: &[PatId],
    body: ExprId,
) -> R<((Vec<Pattern>, Expr), FreeLocals)> {
    let (params, bindings) = pattern::verify(env, DuplicatePatternContext::LambdaArgs, |b| {
        pattern::each(env, module, params, b)
    })?;
    let scope = env.add_locals(&bindings)?;
    let (body, free) = verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))?;
    Ok(((params, body), free))
}

/// `canonicalizeCaseBranch`.
fn case_branch(
    env: &Env<'_>,
    module: &hir::Module,
    pattern: PatId,
    body: ExprId,
) -> R<((Pattern, Expr), FreeLocals)> {
    let (pattern, bindings) = pattern::verify(env, DuplicatePatternContext::CaseBranch, |b| {
        pattern::canonicalize(env, module, pattern, b)
    })?;
    let scope = env.add_locals(&bindings)?;
    let (body, free) = verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))?;
    Ok(((pattern, body), free))
}

/// `findVar`: a local or top-level use counts toward the scope's free
/// locals.
fn find_var(env: &Env<'_>, region: TextRange, name: &str, free: &mut FreeLocals) -> R<ExprKind> {
    match env.vars.get(name) {
        Some(Var::Local(binder)) => {
            env.references.record(region, Target::Local(*binder));
            free.entry(name.to_owned()).or_default().direct += 1;
            Ok(ExprKind::VarLocal(name.to_owned()))
        }
        Some(Var::TopLevel(_)) => {
            env.references.global(region, &env.home, name, Kind::Value);
            free.entry(name.to_owned()).or_default().direct += 1;
            Ok(ExprKind::VarTopLevel(env.home.clone(), name.to_owned()))
        }
        Some(Var::Foreign(homes)) if homes.len() == 1 => {
            let (home, annotation) = homes.iter().next().expect("one home");
            env.references.global(region, home, name, Kind::Value);
            Ok(foreign(env, home, name, annotation))
        }
        Some(Var::Foreign(homes)) => throw(Error::Ambiguous {
            kind: NameKind::Value,
            range: region,
            prefix: None,
            name: name.to_owned(),
            homes: homes.keys().cloned().collect(),
        }),
        None => throw(Error::NotFound {
            kind: NameKind::Value,
            range: region,
            prefix: None,
            name: name.to_owned(),
            possible: possible_vars(env),
        }),
    }
}

/// A foreign value: `Debug`'s by the module using it.
fn foreign(env: &Env<'_>, home: &Home, name: &str, annotation: &Annotation) -> ExprKind {
    if home.package == "elm/core" && home.module == "Debug" {
        ExprKind::VarDebug(env.home.clone(), name.to_owned(), annotation.clone())
    } else {
        ExprKind::VarForeign(home.clone(), name.to_owned(), annotation.clone())
    }
}

/// `findVarQual`, where a kernel module's values are found in kernel
/// packages without importing it.
fn find_var_qual(env: &Env<'_>, region: TextRange, prefix: &str, name: &str) -> R<ExprKind> {
    let not_found = || Error::NotFound {
        kind: NameKind::Value,
        range: region,
        prefix: Some(prefix.to_owned()),
        name: name.to_owned(),
        possible: possible_vars(env),
    };
    match env.q_vars.get(prefix) {
        Some(qualified) => match qualified.get(name) {
            Some(homes) if homes.len() == 1 => {
                let (home, annotation) = homes.iter().next().expect("one home");
                env.references.global(region, home, name, Kind::Value);
                Ok(foreign(env, home, name, annotation))
            }
            Some(homes) => throw(Error::Ambiguous {
                kind: NameKind::Value,
                range: region,
                prefix: Some(prefix.to_owned()),
                name: name.to_owned(),
                homes: homes.keys().cloned().collect(),
            }),
            None => throw(not_found()),
        },
        None if is_kernel_module(prefix) && is_kernel_package(&env.home.package) => {
            let module = prefix.strip_prefix("Elm.Kernel.").unwrap_or(prefix);
            Ok(ExprKind::VarKernel(module.to_owned(), name.to_owned()))
        }
        None => throw(not_found()),
    }
}

fn possible_vars(env: &Env<'_>) -> PossibleNames {
    PossibleNames {
        locals: env.vars.keys().cloned().collect(),
        qualified: (env.q_vars.iter())
            .map(|(prefix, names)| (prefix.clone(), names.keys().cloned().collect()))
            .collect(),
    }
}

/// `canonicalizeLet`: the names every definition binds, each once and
/// shadowing nothing; then each definition, stopping at the first that
/// fails; then the body, and the definitions nested over it in the order
/// the cycle check puts them.
fn let_(
    env: &Env<'_>,
    module: &hir::Module,
    region: TextRange,
    defs: &[DefId],
    body: ExprId,
    free: &mut FreeLocals,
) -> R<Expr> {
    let mut dups = Dict::default();
    for &def in defs {
        match &module.defs[def] {
            Def_::Define { name, .. } => {
                dups.insert(env.compat, &name.text, name.range, name.range)
            }
            Def_::Destruct { pattern, .. } => add_bindings(env, module, *pattern, &mut dups),
        }
    }
    let bindings = dups.detect(env.compat, |name, first, second| Error::DuplicatePattern {
        context: DuplicatePatternContext::LetBinding,
        name: name.to_owned(),
        first,
        second,
    })?;
    let scope = env.add_locals(&bindings)?;
    let (expr, inner) = verify_bindings(&bindings, |let_free| {
        let mut nodes = Vec::new();
        for &def in defs {
            def_node(&scope, module, def, &mut nodes, let_free)?;
        }
        let body = canonicalize(&scope, module, body, let_free)?;
        super::cycles::let_cycles(env.compat, region, nodes, body)
    })?;
    combine(free, inner);
    Ok(expr)
}

/// `addBindingsHelp`: the names a destructuring binds, in upstream's
/// order.
fn add_bindings(env: &Env<'_>, module: &hir::Module, id: PatId, dups: &mut Bindings) {
    let region = module
        .source_map
        .pattern(id)
        .map(|p| p.range)
        .unwrap_or_default();
    match &module.patterns[id] {
        hir::Pattern::Var(name) => dups.insert(env.compat, name, region, region),
        hir::Pattern::Record(fields) => {
            for field in fields {
                dups.insert(env.compat, &field.text, field.range, field.range);
            }
        }
        hir::Pattern::Tuple(items) | hir::Pattern::List(items) => {
            items
                .iter()
                .for_each(|&p| add_bindings(env, module, p, dups));
        }
        hir::Pattern::Ctor { args, .. } => args
            .iter()
            .for_each(|&p| add_bindings(env, module, p, dups)),
        hir::Pattern::Cons(head, tail) => {
            add_bindings(env, module, *head, dups);
            add_bindings(env, module, *tail, dups);
        }
        hir::Pattern::Alias(inner, name) => {
            add_bindings(env, module, *inner, dups);
            dups.insert(env.compat, &name.text, name.range, name.range);
        }
        hir::Pattern::Missing
        | hir::Pattern::Anything
        | hir::Pattern::Unit
        | hir::Pattern::Chr(_)
        | hir::Pattern::Str(_)
        | hir::Pattern::Int(_) => {}
    }
}

/// `addDefNodes`: one definition of a `let`, canonicalized, as nodes of
/// the `let`'s graph, its uses counted toward the `let`'s.
fn def_node(
    env: &Env<'_>,
    module: &hir::Module,
    def: DefId,
    nodes: &mut Vec<LetNode>,
    let_free: &mut FreeLocals,
) -> R<()> {
    match &module.defs[def] {
        Def_::Define {
            name,
            params,
            body,
            annotation,
            ..
        } => {
            let (cdef, inner) = definition(env, module, name, params, *body, *annotation)?;
            let uses: Vec<String> = inner.keys().cloned().collect();
            nodes.push((LetBinding::Define(cdef), name.text.clone(), uses));
            combine(
                let_free,
                if params.is_empty() {
                    inner
                } else {
                    delay(inner)
                },
            );
            Ok(())
        }
        Def_::Destruct { pattern, body } => {
            let (cpattern, _) = pattern::verify(env, DuplicatePatternContext::Destruct, |b| {
                pattern::canonicalize(env, module, *pattern, b)
            })?;
            let mut inner = FreeLocals::new();
            let cbody = canonicalize(env, module, *body, &mut inner)?;
            let names = pattern_names(env.compat, module, *pattern);
            // `Name.fromManyNames`: `_M$` and the first name, which no
            // name of the source can be.
            let key = names
                .first()
                .map_or(String::new(), |(n, _)| format!("_M${n}"));
            let uses = inner.keys().cloned().collect();
            combine(let_free, inner);
            nodes.push((LetBinding::Destruct(cpattern, cbody), key.clone(), uses));
            for (name, range) in names {
                nodes.push((
                    LetBinding::Edge {
                        name: name.clone(),
                        range,
                    },
                    name,
                    vec![key.clone()],
                ));
            }
            Ok(())
        }
    }
}

/// A definition's parameters and body, against its annotation where it
/// has one (`toNodeOne` and `addDefNodes` alike), and the locals its body
/// uses from outside it.
pub fn definition(
    env: &Env<'_>,
    module: &hir::Module,
    name: &hir::Spanned,
    params: &[PatId],
    body: ExprId,
    annotation: Option<hir::TypeId>,
) -> R<(CDef, FreeLocals)> {
    let context = DuplicatePatternContext::FuncArgs(name.text.clone());
    match annotation {
        None => {
            let (params, bindings) =
                pattern::verify(env, context, |b| pattern::each(env, module, params, b))?;
            let scope = env.add_locals(&bindings)?;
            let (body, free) =
                verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))?;
            let def = CDef::Def {
                name: name.text.clone(),
                region: name.range,
                params,
                body,
            };
            Ok((def, free))
        }
        Some(ty) => {
            let annotation = types::to_annotation(env, module, ty)?;
            let ((params, result), bindings) = pattern::verify(env, context, |b| {
                super::module::typed_args(env, module, &name.text, params, &annotation.ty, b)
            })?;
            let scope = env.add_locals(&bindings)?;
            let (body, free) =
                verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))?;
            let def = CDef::TypedDef {
                name: name.text.clone(),
                region: name.range,
                free: annotation.free,
                params,
                body,
                result,
            };
            Ok((def, free))
        }
    }
}

/// `getPatternNames`, in the order it lists them. 0.19.1's parser keeps
/// a record pattern's fields last first.
fn pattern_names(compat: Compat, module: &hir::Module, id: PatId) -> Vec<(String, TextRange)> {
    fn go(compat: Compat, module: &hir::Module, id: PatId, names: &mut Vec<(String, TextRange)>) {
        let region = module
            .source_map
            .pattern(id)
            .map(|p| p.range)
            .unwrap_or_default();
        match &module.patterns[id] {
            hir::Pattern::Var(name) => names.insert(0, (name.clone(), region)),
            hir::Pattern::Record(fields) => {
                let mut new: Vec<_> = fields.iter().map(|f| (f.text.clone(), f.range)).collect();
                if compat == Compat::V0_19_1 {
                    new.reverse();
                }
                new.append(names);
                *names = new;
            }
            hir::Pattern::Alias(inner, name) => {
                names.insert(0, (name.text.clone(), name.range));
                go(compat, module, *inner, names);
            }
            hir::Pattern::Tuple(items) | hir::Pattern::List(items) => {
                items.iter().for_each(|&p| go(compat, module, p, names))
            }
            hir::Pattern::Ctor { args, .. } => {
                args.iter().for_each(|&p| go(compat, module, p, names))
            }
            hir::Pattern::Cons(head, tail) => {
                go(compat, module, *head, names);
                go(compat, module, *tail, names);
            }
            hir::Pattern::Missing
            | hir::Pattern::Anything
            | hir::Pattern::Unit
            | hir::Pattern::Chr(_)
            | hir::Pattern::Str(_)
            | hir::Pattern::Int(_) => {}
        }
    }
    let mut names = Vec::new();
    go(compat, module, id, &mut names);
    names
}
