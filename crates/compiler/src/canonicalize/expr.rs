//! Canonicalizing expressions (`Canonicalize/Expression.hs`): each
//! variable, constructor and operator resolved, every scope's bindings
//! checked, and the locals each scope uses counted (`FreeLocals`), which
//! the cycle checks of plan 5.12 read. A scope's uses count only when it
//! canonicalized cleanly, as upstream discards them otherwise.

use std::collections::BTreeMap;

use nomi_base::TextRange;

use super::binops;
use super::dups::{Dict, check_fields};
use super::env::{Env, Var};
use super::error::{DuplicatePatternContext, Error, NameKind, PossibleNames, R, all, both, throw};
use super::foreign::{is_kernel_module, is_kernel_package};
use super::pattern::{self, Bindings};
use super::references::{Kind, Target};
use super::types;
use crate::hir::{self, Def, DefId, Expr, ExprId, PatId, Pattern};

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
    Define {
        name: String,
        range: TextRange,
        has_params: bool,
    },
    /// A name a destructuring binds.
    Edge {
        name: String,
        range: TextRange,
    },
    Destruct,
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

/// `canonicalize`.
pub fn canonicalize(
    env: &Env<'_>,
    module: &hir::Module,
    id: ExprId,
    free: &mut FreeLocals,
) -> R<()> {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
        canonicalize_(env, module, id, free)
    })
}

fn each(env: &Env<'_>, module: &hir::Module, ids: &[ExprId], free: &mut FreeLocals) -> R<()> {
    all(ids.iter().map(|&id| canonicalize(env, module, id, free))).map(|_| ())
}

fn canonicalize_(env: &Env<'_>, module: &hir::Module, id: ExprId, free: &mut FreeLocals) -> R<()> {
    let region = range(module, id);
    match &module.exprs[id] {
        Expr::Missing
        | Expr::Chr(_)
        | Expr::Str(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Accessor(_)
        | Expr::Unit
        | Expr::Shader(_) => Ok(()),
        Expr::Var {
            qualifier,
            name,
            upper: false,
        } => match qualifier {
            None => find_var(env, region, name, free),
            Some(prefix) => find_var_qual(env, region, prefix, name),
        },
        Expr::Var {
            qualifier, name, ..
        } => env
            .find_ctor(region, qualifier.as_deref(), name)
            .map(|_| ()),
        Expr::List(items) => each(env, module, items, free),
        Expr::Op(op) => env.find_binop(region, op).map(|_| ()),
        Expr::Negate(inner) => canonicalize(env, module, *inner, free),
        Expr::Binops { operands, ops } => {
            // Each operand with the operator after it, then the last, and
            // only then the chain arranged by precedence.
            let chain = all(operands
                .iter()
                .zip(ops)
                .enumerate()
                .map(|(i, (&operand, op))| {
                    let operand = canonicalize(env, module, operand, free);
                    let found = env.find_binop(op.range, &op.text).map(|b| binops::Op {
                        index: i,
                        name: op.text.clone(),
                        associativity: b.associativity,
                        precedence: b.precedence,
                    });
                    both(operand, found).map(|((), op)| (binops::Tree::Operand(i), op))
                }));
            let last = canonicalize(env, module, *operands.last().expect("an operand"), free);
            let (chain, ()) = both(chain, last)?;
            binops::arrange(chain, binops::Tree::Operand(ops.len()), region).map(|_| ())
        }
        Expr::Lambda { params, body } => {
            let (_, inner) = lambda(env, module, params, *body)?;
            combine(free, delay(inner));
            Ok(())
        }
        Expr::Call { function, args } => {
            let function = canonicalize(env, module, *function, free);
            let args = each(env, module, args, free);
            both(function, args).map(|_| ())
        }
        Expr::If {
            branches,
            otherwise,
        } => {
            let branches = all(branches.iter().map(|&(condition, then)| {
                let condition = canonicalize(env, module, condition, free);
                let then = canonicalize(env, module, then, free);
                both(condition, then)
            }));
            let otherwise = canonicalize(env, module, *otherwise, free);
            both(branches, otherwise).map(|_| ())
        }
        Expr::Let { defs, body } => let_(env, module, defs, *body, free),
        Expr::Case {
            scrutinee,
            branches,
        } => {
            let scrutinee = canonicalize(env, module, *scrutinee, free);
            let branches = all(branches.iter().map(|&(pattern, body)| {
                let (_, inner) = case_branch(env, module, pattern, body)?;
                combine(free, inner);
                Ok(())
            }));
            both(scrutinee, branches).map(|_| ())
        }
        Expr::Access { record, .. } => canonicalize(env, module, *record, free),
        Expr::Update { record, fields } => {
            let found = find_var(env, record.range, &record.text, free);
            let fields = (fields.iter())
                .map(|(name, value)| (name.text.clone(), name.range, *value))
                .collect();
            let fields = check_fields(env.compat, fields).and_then(|fields| {
                let names = super::sorted(env.compat, fields.keys());
                all(names
                    .into_iter()
                    .map(|n| canonicalize(env, module, fields[n], free)))
                .map(|_| ())
            });
            both(found, fields).map(|_| ())
        }
        Expr::Record(fields) => {
            let fields = (fields.iter())
                .map(|(name, value)| (name.text.clone(), name.range, *value))
                .collect();
            let fields = check_fields(env.compat, fields)?;
            let names = super::sorted(env.compat, fields.keys());
            all(names
                .into_iter()
                .map(|n| canonicalize(env, module, fields[n], free)))
            .map(|_| ())
        }
        Expr::Tuple(items) => {
            let a = canonicalize(env, module, items[0], free);
            let b = canonicalize(env, module, items[1], free);
            let rest = match &items[2..] {
                [] => Ok(()),
                [c] => canonicalize(env, module, *c, free),
                _ => throw(Error::TupleLargerThanThree { range: region }),
            };
            both(both(a, b), rest).map(|_| ())
        }
    }
}

/// A lambda: its parameters, bound once each, then its body.
fn lambda(
    env: &Env<'_>,
    module: &hir::Module,
    params: &[PatId],
    body: ExprId,
) -> R<((), FreeLocals)> {
    let ((), bindings) = pattern::verify(env, DuplicatePatternContext::LambdaArgs, |b| {
        all(params
            .iter()
            .map(|&p| pattern::canonicalize(env, module, p, b)))
        .map(|_| ())
    })?;
    let scope = env.add_locals(&bindings)?;
    verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))
}

/// `canonicalizeCaseBranch`.
fn case_branch(
    env: &Env<'_>,
    module: &hir::Module,
    pattern: PatId,
    body: ExprId,
) -> R<((), FreeLocals)> {
    let ((), bindings) = pattern::verify(env, DuplicatePatternContext::CaseBranch, |b| {
        pattern::canonicalize(env, module, pattern, b)
    })?;
    let scope = env.add_locals(&bindings)?;
    verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))
}

/// `findVar`: a local or top-level use counts toward the scope's free
/// locals.
fn find_var(env: &Env<'_>, region: TextRange, name: &str, free: &mut FreeLocals) -> R<()> {
    match env.vars.get(name) {
        Some(var @ (Var::Local(_) | Var::TopLevel(_))) => {
            match var {
                Var::Local(binder) => env.references.record(region, Target::Local(*binder)),
                _ => env.references.global(region, &env.home, name, Kind::Value),
            }
            free.entry(name.to_owned()).or_default().direct += 1;
            Ok(())
        }
        Some(Var::Foreign(homes)) if homes.len() == 1 => {
            let home = homes.keys().next().expect("one home");
            env.references.global(region, home, name, Kind::Value);
            Ok(())
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

/// `findVarQual`, where a kernel module's values are found in kernel
/// packages without importing it.
fn find_var_qual(env: &Env<'_>, region: TextRange, prefix: &str, name: &str) -> R<()> {
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
                let home = homes.keys().next().expect("one home");
                env.references.global(region, home, name, Kind::Value);
                Ok(())
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
        None if is_kernel_module(prefix) && is_kernel_package(&env.home.package) => Ok(()),
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
/// fails; then the body.
fn let_(
    env: &Env<'_>,
    module: &hir::Module,
    defs: &[DefId],
    body: ExprId,
    free: &mut FreeLocals,
) -> R<()> {
    let mut dups = Dict::default();
    for &def in defs {
        match &module.defs[def] {
            Def::Define { name, .. } => dups.insert(env.compat, &name.text, name.range, name.range),
            Def::Destruct { pattern, .. } => add_bindings(env, module, *pattern, &mut dups),
        }
    }
    let bindings = dups.detect(env.compat, |name, first, second| Error::DuplicatePattern {
        context: DuplicatePatternContext::LetBinding,
        name: name.to_owned(),
        first,
        second,
    })?;
    let scope = env.add_locals(&bindings)?;
    let ((), inner) = verify_bindings(&bindings, |let_free| {
        let mut nodes = Vec::new();
        for &def in defs {
            def_node(&scope, module, def, &mut nodes, let_free)?;
        }
        canonicalize(&scope, module, body, let_free)?;
        super::cycles::let_cycles(env.compat, nodes)
    })?;
    combine(free, inner);
    Ok(())
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
        Pattern::Var(name) => dups.insert(env.compat, name, region, region),
        Pattern::Record(fields) => {
            for field in fields {
                dups.insert(env.compat, &field.text, field.range, field.range);
            }
        }
        Pattern::Tuple(items) | Pattern::List(items) => {
            items
                .iter()
                .for_each(|&p| add_bindings(env, module, p, dups));
        }
        Pattern::Ctor { args, .. } => args
            .iter()
            .for_each(|&p| add_bindings(env, module, p, dups)),
        Pattern::Cons(head, tail) => {
            add_bindings(env, module, *head, dups);
            add_bindings(env, module, *tail, dups);
        }
        Pattern::Alias(inner, name) => {
            add_bindings(env, module, *inner, dups);
            dups.insert(env.compat, &name.text, name.range, name.range);
        }
        Pattern::Missing
        | Pattern::Anything
        | Pattern::Unit
        | Pattern::Chr(_)
        | Pattern::Str(_)
        | Pattern::Int(_) => {}
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
        Def::Define {
            name,
            params,
            body,
            annotation,
            ..
        } => {
            let inner = definition(env, module, &name.text, params, *body, *annotation)?;
            let uses: Vec<String> = inner.keys().cloned().collect();
            let binding = LetBinding::Define {
                name: name.text.clone(),
                range: name.range,
                has_params: !params.is_empty(),
            };
            nodes.push((binding, name.text.clone(), uses));
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
        Def::Destruct { pattern, body } => {
            pattern::verify(env, DuplicatePatternContext::Destruct, |b| {
                pattern::canonicalize(env, module, *pattern, b)
            })?;
            let mut inner = FreeLocals::new();
            canonicalize(env, module, *body, &mut inner)?;
            let names = pattern_names(module, *pattern);
            // `Name.fromManyNames`: `_M$` and the first name, which no
            // name of the source can be.
            let key = names
                .first()
                .map_or(String::new(), |(n, _)| format!("_M${n}"));
            let uses = inner.keys().cloned().collect();
            combine(let_free, inner);
            nodes.push((LetBinding::Destruct, key.clone(), uses));
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
/// has one (`toNodeOne` and `addDefNodes` alike): the locals its body
/// uses from outside it.
pub fn definition(
    env: &Env<'_>,
    module: &hir::Module,
    name: &str,
    params: &[PatId],
    body: ExprId,
    annotation: Option<hir::TypeId>,
) -> R<FreeLocals> {
    let context = DuplicatePatternContext::FuncArgs(name.to_owned());
    let ((), bindings) = match annotation {
        None => pattern::verify(env, context, |b| {
            all(params
                .iter()
                .map(|&p| pattern::canonicalize(env, module, p, b)))
            .map(|_| ())
        })?,
        Some(ty) => {
            let annotation = types::to_annotation(env, module, ty)?;
            pattern::verify(env, context, |b| {
                super::module::typed_args(env, module, name, params, &annotation.ty, b)
            })?
        }
    };
    let scope = env.add_locals(&bindings)?;
    let ((), free) = verify_bindings(&bindings, |free| canonicalize(&scope, module, body, free))?;
    Ok(free)
}

/// `getPatternNames`, in the order it lists them.
fn pattern_names(module: &hir::Module, id: PatId) -> Vec<(String, TextRange)> {
    fn go(module: &hir::Module, id: PatId, names: &mut Vec<(String, TextRange)>) {
        let region = module
            .source_map
            .pattern(id)
            .map(|p| p.range)
            .unwrap_or_default();
        match &module.patterns[id] {
            Pattern::Var(name) => names.insert(0, (name.clone(), region)),
            Pattern::Record(fields) => {
                let mut new: Vec<_> = fields.iter().map(|f| (f.text.clone(), f.range)).collect();
                new.append(names);
                *names = new;
            }
            Pattern::Alias(inner, name) => {
                names.insert(0, (name.text.clone(), name.range));
                go(module, *inner, names);
            }
            Pattern::Tuple(items) | Pattern::List(items) => {
                items.iter().for_each(|&p| go(module, p, names))
            }
            Pattern::Ctor { args, .. } => args.iter().for_each(|&p| go(module, p, names)),
            Pattern::Cons(head, tail) => {
                go(module, *head, names);
                go(module, *tail, names);
            }
            Pattern::Missing
            | Pattern::Anything
            | Pattern::Unit
            | Pattern::Chr(_)
            | Pattern::Str(_)
            | Pattern::Int(_) => {}
        }
    }
    let mut names = Vec::new();
    go(module, id, &mut names);
    names
}
