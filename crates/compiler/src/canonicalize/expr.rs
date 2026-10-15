//! Canonicalizing expressions (`Canonicalize/Expression.hs`): each
//! variable, constructor and operator resolved, every scope's bindings
//! checked, and the locals each scope uses noted (`FreeLocals`), which
//! the cycle checks read. A scope's uses count only when it
//! canonicalized cleanly, as upstream discards them otherwise.

use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange, grow, grow_large};

use super::binops;
use super::dups::{Dict, check_fields};
use super::env::{self, BinopInfo, CtorInfo, Env, Var};
use super::error::{DuplicatePatternContext, Error, NameKind, R, all, both, throw};
use super::foreign::{is_kernel_module, is_kernel_package};
use super::pattern::{self, Bindings};
use super::references::{Kind, Target};
use super::types;
use crate::canonical::{
    Annotation, CtorOpts, Def, Expr, ExprKind, FieldUpdate, Home, Operator, Pattern, Type,
};
use crate::graph::Node;
use crate::hir::{self, DefId, ExprId, PatId, Spanned, VarType, int_value};

/// How a local is used, which the cycle checks read (`Uses`): somewhere
/// directly, or only inside functions that may run later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Use {
    Delayed,
    Direct,
}

/// `FreeLocals`.
pub(super) type FreeLocals = BTreeMap<String, Use>;

/// `Map.unionWith combineUses`, into `into`.
fn combine(into: &mut FreeLocals, from: FreeLocals) {
    for (name, used) in from {
        let entry = into.entry(name).or_insert(used);
        *entry = (*entry).max(used);
    }
}

/// `delayUse`: every use as delayed.
fn delay(mut free: FreeLocals) -> FreeLocals {
    for used in free.values_mut() {
        *used = Use::Delayed;
    }
    free
}

/// `verifyBindings`: what `work` makes in `env` with `bindings` added,
/// and the locals it uses from outside the scope.
fn verify_bindings<'a, T>(
    env: &Env<'a>,
    bindings: &BTreeMap<String, TextRange>,
    work: impl FnOnce(&Env<'a>, &mut FreeLocals) -> R<T>,
) -> R<(T, FreeLocals)> {
    let scope = env.add_locals(bindings)?;
    let mut free = FreeLocals::new();
    let value = work(&scope, &mut free)?;
    free.retain(|name, _| !bindings.contains_key(name));
    Ok((value, free))
}

/// One binding of a `let`, as the cycle check sees it (`Binding`).
#[derive(Clone, Debug)]
pub(super) enum LetBinding {
    Define(Def),
    /// A name a destructuring binds.
    Edge(Spanned),
    Destruct(Pattern, Expr),
}

fn expr(region: TextRange, kind: ExprKind) -> Expr {
    Expr { region, kind }
}

/// `canonicalize`.
fn canonicalize(env: &Env<'_>, module: &hir::Module, id: ExprId, free: &mut FreeLocals) -> R<Expr> {
    grow_large(|| canonicalize_(env, module, id, free))
}

fn each(
    env: &Env<'_>,
    module: &hir::Module,
    ids: &[ExprId],
    free: &mut FreeLocals,
) -> R<Vec<Expr>> {
    all(ids.iter().map(|&id| canonicalize(env, module, id, free)))
}

#[expect(
    clippy::too_many_lines,
    reason = "upstream's `canonicalize`, one arm per expression"
)]
fn canonicalize_(
    env: &Env<'_>,
    module: &hir::Module,
    id: ExprId,
    free: &mut FreeLocals,
) -> R<Expr> {
    let region = module.source_map.expr_range(id);
    let kind = match &module.exprs[id] {
        // A module with syntax errors canonicalizes for the editor only.
        hir::Expr::Missing | hir::Expr::Unit => ExprKind::Unit,
        hir::Expr::Chr(text) => ExprKind::Chr(text.clone()),
        hir::Expr::Str(text) => ExprKind::Str(text.clone()),
        hir::Expr::Int(text) => ExprKind::Int(int_value(text)),
        hir::Expr::Float(text) => ExprKind::Float(text.clone()),
        hir::Expr::Accessor(field) => ExprKind::Accessor(field.clone()),
        hir::Expr::Shader(source) => ExprKind::Shader(source.clone(), super::shader::types(source)),
        hir::Expr::Var {
            qualifier,
            name,
            kind: VarType::Low,
        } => match qualifier {
            None => find_var(env, region, name, free)?,
            Some(prefix) => find_var_qual(env, region, prefix, name)?,
        },
        hir::Expr::Var {
            qualifier,
            name,
            kind: VarType::Cap,
        } => to_var_ctor(name, env.find_ctor(region, qualifier.as_deref(), name)?),
        hir::Expr::List(items) => ExprKind::List(each(env, module, items, free)?),
        hir::Expr::Op(op) => ExprKind::VarOperator(operator(env.find_binop(region, op)?)),
        hir::Expr::Negate(inner) => {
            ExprKind::Negate(Box::new(canonicalize(env, module, *inner, free)?))
        }
        hir::Expr::Binops { operands, ops } => {
            return binop_chain(env, module, region, operands, ops, free);
        }
        hir::Expr::Lambda { params, body } => {
            let context = DuplicatePatternContext::LambdaArgs;
            let each = |b: &mut Bindings| pattern::each(env, module, params, b);
            let lambda = scoped(env, module, &context, each, *body)?;
            combine(free, delay(lambda.free));
            ExprKind::Lambda(lambda.patterns, Box::new(lambda.body))
        }
        hir::Expr::Call { function, args } => {
            let function = canonicalize(env, module, *function, free);
            let args = each(env, module, args, free);
            let (function, args) = both(function, args)?;
            // `A.merge func lastArg`: a parenthesized argument's region is
            // its inner expression's, so the call ends inside the paren.
            let end = args.last().map_or_else(|| region.end(), |a| a.region.end());
            let region = TextRange::new(function.region.start(), end);
            return Ok(expr(region, ExprKind::Call(Box::new(function), args)));
        }
        hir::Expr::If {
            branches,
            otherwise,
        } => {
            let branches = all(branches.iter().map(|&(condition, then)| {
                let condition = canonicalize(env, module, condition, free);
                let then = canonicalize(env, module, then, free);
                both(condition, then)
            }));
            let otherwise = canonicalize(env, module, *otherwise, free);
            let (branches, otherwise) = both(branches, otherwise)?;
            ExprKind::If(branches, Box::new(otherwise))
        }
        hir::Expr::Let { defs, body } => return let_(env, module, region, defs, *body, free),
        hir::Expr::Case {
            scrutinee,
            branches,
        } => {
            let scrutinee = canonicalize(env, module, *scrutinee, free);
            let branches = all(branches.iter().map(|&(pattern, body)| {
                // `canonicalizeCaseBranch`.
                let context = DuplicatePatternContext::CaseBranch;
                let one = |b: &mut Bindings| pattern::canonicalize(env, module, pattern, b);
                let branch = scoped(env, module, &context, one, body)?;
                combine(free, branch.free);
                Ok((branch.patterns, branch.body))
            }));
            let (scrutinee, branches) = both(scrutinee, branches)?;
            ExprKind::Case(Box::new(scrutinee), branches)
        }
        hir::Expr::Access { record, field } => ExprKind::Access {
            record: Box::new(canonicalize(env, module, *record, free)?),
            field: field.text.clone(),
            field_region: field.range,
        },
        hir::Expr::Update { record, fields } => update(env, module, record, fields, free)?,
        hir::Expr::Record(fields) => {
            let fields = check_fields(
                env.compat,
                fields.iter().map(|(name, value)| (name, *value)),
            )?;
            let names = env.compat.sorted_names(fields.keys());
            let values = all(names
                .iter()
                .map(|n| canonicalize(env, module, fields[*n], free).map(|v| ((*n).clone(), v))))?;
            ExprKind::Record(values.into_iter().collect())
        }
        hir::Expr::Tuple(items) => {
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

/// `toBinop`'s operator: the one `binop` names, and the function it
/// stands for.
fn operator(binop: &BinopInfo<'_>) -> Operator {
    Operator {
        op: binop.op.clone(),
        home: binop.home.clone(),
        function: binop.function.clone(),
        annotation: binop.annotation.clone(),
    }
}

/// A chain of operators: each operand with the operator after it, then
/// the last, and only then the chain arranged by precedence.
fn binop_chain(
    env: &Env<'_>,
    module: &hir::Module,
    region: TextRange,
    operands: &[ExprId],
    ops: &[Spanned],
    free: &mut FreeLocals,
) -> R<Expr> {
    let chain = all((operands.iter().zip(ops)).map(|(&operand, op)| {
        let operand = canonicalize(env, module, operand, free);
        both(operand, env.find_binop(op.range, &op.text))
    }));
    let last = canonicalize(env, module, *operands.last().expect("an operand"), free);
    let (chain, last) = both(chain, last)?;
    let found: Vec<&BinopInfo<'_>> = chain.iter().map(|&(_, binop)| binop).collect();
    let chain = (chain.into_iter()).map(|(operand, binop)| {
        let op = binops::Op {
            name: &binop.op,
            fixity: binop.fixity,
        };
        (operand, op)
    });
    // Each node's region runs from its left operand's start to its right
    // one's end (`A.merge`); the chain's root keeps the chain's own
    // region, as upstream wraps the stepper's result in it.
    let mut root = binops::arrange(chain, last, region, |at, left: Expr, right: Expr| {
        let region = left.region.cover(right.region);
        let kind = ExprKind::Binop {
            operator: operator(found[at]),
            left: Box::new(left),
            right: Box::new(right),
        };
        expr(region, kind)
    })?;
    root.region = region;
    Ok(root)
}

/// A record update: the record's variable, then each field written once,
/// canonicalized in name order.
fn update(
    env: &Env<'_>,
    module: &hir::Module,
    record: &Spanned,
    fields: &[(Spanned, ExprId)],
    free: &mut FreeLocals,
) -> R<ExprKind> {
    let found =
        find_var(env, record.range, &record.text, free).map(|kind| expr(record.range, kind));
    let fields = (fields.iter()).map(|(name, value)| (name, (name.range, *value)));
    let fields = check_fields(env.compat, fields).and_then(|fields| {
        let names = env.compat.sorted_names(fields.keys());
        let values = all(names.iter().map(|n| {
            let (region, value) = fields[*n];
            canonicalize(env, module, value, free)
                .map(|value| ((*n).clone(), FieldUpdate { region, value }))
        }));
        values.map(|v| v.into_iter().collect::<BTreeMap<_, _>>())
    });
    let (found, fields) = both(found, fields)?;
    Ok(ExprKind::Update {
        name: record.text.clone(),
        record: Box::new(found),
        fields,
    })
}

/// `toVarCtor`: a constructor, with its type.
fn to_var_ctor(name: &str, ctor: &CtorInfo<'_>) -> ExprKind {
    let (opts, home, index, annotation) = match ctor {
        CtorInfo::Ctor {
            home,
            union_name,
            union,
            ctor: index,
        } => {
            let ctor = &union.ctors[*index];
            let result = Type::Type {
                home: home.clone(),
                name: union_name.clone(),
                args: union.vars.iter().map(|v| Type::Var(v.clone())).collect(),
            };
            let annotation = Annotation {
                free: sorted_vars(&union.vars),
                ty: Type::function(ctor.args.iter().cloned(), result).into(),
            };
            (union.opts, home, *index, annotation)
        }
        CtorInfo::Record { home, vars, ty } => {
            let annotation = Annotation {
                free: sorted_vars(vars),
                ty: ty.clone().into(),
            };
            (CtorOpts::Normal, home, 0, annotation)
        }
    };
    ExprKind::VarCtor {
        opts,
        home: home.clone(),
        name: name.to_owned(),
        index,
        annotation,
    }
}

/// A type's variables as a set, as `Map.fromList` keeps them.
fn sorted_vars(vars: &[String]) -> Vec<String> {
    let mut vars = vars.to_vec();
    vars.sort();
    vars.dedup();
    vars
}

/// Patterns, and a body canonicalized in their scope.
struct Scoped<P> {
    patterns: P,
    body: Expr,
    /// The locals the body uses from outside the scope.
    free: FreeLocals,
}

/// What `patterns` makes, each name it binds bound once under `context`,
/// then `body` in their scope.
fn scoped<P>(
    env: &Env<'_>,
    module: &hir::Module,
    context: &DuplicatePatternContext,
    patterns: impl FnOnce(&mut Bindings) -> R<P>,
    body: ExprId,
) -> R<Scoped<P>> {
    let (patterns, bindings) = pattern::verify(env.compat, context, patterns)?;
    let (body, free) = verify_bindings(env, &bindings, |scope, free| {
        canonicalize(scope, module, body, free)
    })?;
    Ok(Scoped {
        patterns,
        body,
        free,
    })
}

/// `findVar`: a local or top-level use counts toward the scope's free
/// locals.
fn find_var(env: &Env<'_>, region: TextRange, name: &str, free: &mut FreeLocals) -> R<ExprKind> {
    match env.vars.get(name) {
        Some(Var::Local(binder)) => {
            env.references.record(region, || Target::Local(*binder));
            free.insert(name.to_owned(), Use::Direct);
            Ok(ExprKind::VarLocal(name.to_owned()))
        }
        Some(Var::TopLevel(_)) => {
            env.references.global(region, &env.home, name, Kind::Value);
            free.insert(name.to_owned(), Use::Direct);
            Ok(ExprKind::VarTopLevel(env.home.clone(), name.to_owned()))
        }
        Some(Var::Foreign(homes)) => foreign_var(env, region, None, name, homes),
        None => throw(not_found(env, region, None, name)),
    }
}

/// `findVarQual`, where a kernel module's values are found in kernel
/// packages without importing it.
fn find_var_qual(env: &Env<'_>, region: TextRange, prefix: &str, name: &str) -> R<ExprKind> {
    match env.q_vars.get(prefix) {
        Some(qualified) => match qualified.get(name) {
            Some(homes) => foreign_var(env, region, Some(prefix), name, homes),
            None => throw(not_found(env, region, Some(prefix), name)),
        },
        None if is_kernel_module(prefix) && is_kernel_package(&env.home.package) => {
            Ok(ExprKind::VarKernel {
                module: prefix
                    .strip_prefix("Elm.Kernel.")
                    .unwrap_or(prefix)
                    .to_owned(),
                name: name.to_owned(),
            })
        }
        None => throw(not_found(env, region, Some(prefix), name)),
    }
}

/// A foreign value, if one import offers it: `Debug`'s by the module
/// using it.
fn foreign_var(
    env: &Env<'_>,
    region: TextRange,
    prefix: Option<&str>,
    name: &str,
    homes: &BTreeMap<Home, &Annotation>,
) -> R<ExprKind> {
    let (home, annotation) = env::one(homes).or_else(|homes| {
        throw(Error::Ambiguous {
            kind: NameKind::Value,
            range: region,
            prefix: prefix.map(str::to_owned),
            name: name.to_owned(),
            homes,
        })
    })?;
    env.references.global(region, home, name, Kind::Value);
    let annotation = (*annotation).clone();
    Ok(if home.is("elm/core", "Debug") {
        ExprKind::VarDebug {
            home: env.home.clone(),
            name: name.to_owned(),
            annotation,
        }
    } else {
        ExprKind::VarForeign {
            home: home.clone(),
            name: name.to_owned(),
            annotation,
        }
    })
}

fn not_found(env: &Env<'_>, range: TextRange, prefix: Option<&str>, name: &str) -> Error {
    Error::NotFound {
        kind: NameKind::Value,
        range,
        prefix: prefix.map(str::to_owned),
        name: name.to_owned(),
        possible: env::possible(env.vars.keys(), &env.q_vars),
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
            hir::Def::Define { name, .. } => {
                dups.insert(env.compat, &name.text, name.range, name.range);
            }
            hir::Def::Destruct { pattern, .. } => {
                add_bindings(env.compat, module, *pattern, &mut dups);
            }
        }
    }
    let bindings = dups.detect(env.compat, |name, first, second| Error::DuplicatePattern {
        context: DuplicatePatternContext::LetBinding,
        name: name.to_owned(),
        first,
        second,
    })?;
    let (expr, inner) = verify_bindings(env, &bindings, |scope, let_free| {
        let mut nodes = Vec::new();
        for &def in defs {
            def_node(scope, module, def, &mut nodes, let_free)?;
        }
        let body = canonicalize(scope, module, body, let_free)?;
        super::cycles::let_cycles(env.compat, region, nodes, body)
    })?;
    combine(free, inner);
    Ok(expr)
}

/// `addBindingsHelp`: the names a destructuring binds, in upstream's
/// order.
fn add_bindings(compat: Compat, module: &hir::Module, id: PatId, dups: &mut Bindings) {
    grow(|| match &module.patterns[id] {
        hir::Pattern::Var(name) => {
            let region = module.source_map.pattern_range(id);
            dups.insert(compat, name, region, region);
        }
        hir::Pattern::Record(fields) => {
            for field in fields {
                dups.insert(compat, &field.text, field.range, field.range);
            }
        }
        hir::Pattern::Tuple(items) | hir::Pattern::List(items) => {
            for &p in items {
                add_bindings(compat, module, p, dups);
            }
        }
        hir::Pattern::Ctor { args, .. } => {
            for &p in args {
                add_bindings(compat, module, p, dups);
            }
        }
        hir::Pattern::Cons(head, tail) => {
            add_bindings(compat, module, *head, dups);
            add_bindings(compat, module, *tail, dups);
        }
        hir::Pattern::Alias(inner, name) => {
            add_bindings(compat, module, *inner, dups);
            dups.insert(compat, &name.text, name.range, name.range);
        }
        hir::Pattern::Missing
        | hir::Pattern::Anything
        | hir::Pattern::Unit
        | hir::Pattern::Chr(_)
        | hir::Pattern::Str(_)
        | hir::Pattern::Int(_) => {}
    });
}

/// `addDefNodes`: one definition of a `let`, canonicalized, as nodes of
/// the `let`'s graph, its uses counted toward the `let`'s.
fn def_node(
    env: &Env<'_>,
    module: &hir::Module,
    def: DefId,
    nodes: &mut Vec<Node<LetBinding>>,
    let_free: &mut FreeLocals,
) -> R<()> {
    match &module.defs[def] {
        hir::Def::Define {
            name,
            params,
            body,
            annotation,
            ..
        } => {
            let defined = definition(env, module, name, params, *body, *annotation)?;
            nodes.push(Node {
                value: LetBinding::Define(defined.def),
                key: name.text.clone(),
                edges: env.compat.sorted_names(defined.free.keys().cloned()),
            });
            let free = if params.is_empty() {
                defined.free
            } else {
                delay(defined.free)
            };
            combine(let_free, free);
            Ok(())
        }
        hir::Def::Destruct { pattern, body } => {
            let context = DuplicatePatternContext::Destruct;
            let (cpattern, _) = pattern::verify(env.compat, &context, |b| {
                pattern::canonicalize(env, module, *pattern, b)
            })?;
            let mut inner = FreeLocals::new();
            let cbody = canonicalize(env, module, *body, &mut inner)?;
            let names = pattern_names(env.compat, module, *pattern);
            // `Name.fromManyNames`: `_M$` and the first name, which no
            // name of the source can be.
            let key = names
                .first()
                .map_or(String::new(), |n| format!("_M${}", n.text));
            let edges = env.compat.sorted_names(inner.keys().cloned());
            combine(let_free, inner);
            nodes.push(Node {
                value: LetBinding::Destruct(cpattern, cbody),
                key: key.clone(),
                edges,
            });
            for name in names {
                nodes.push(Node {
                    key: name.text.clone(),
                    value: LetBinding::Edge(name),
                    edges: vec![key.clone()],
                });
            }
            Ok(())
        }
    }
}

/// A definition canonicalized, with what its annotation says where it has
/// one.
pub(super) struct Defined {
    pub(super) def: Def,
    pub(super) annotation: Option<Annotation>,
    /// The locals its body uses from outside it.
    pub(super) free: FreeLocals,
}

/// A definition's parameters and body, against its annotation where it
/// has one (`toNodeOne` and `addDefNodes` alike).
pub(super) fn definition(
    env: &Env<'_>,
    module: &hir::Module,
    name: &Spanned,
    params: &[PatId],
    body: ExprId,
    annotation: Option<hir::TypeId>,
) -> R<Defined> {
    let context = DuplicatePatternContext::FuncArgs(name.text.clone());
    match annotation {
        None => {
            let each = |b: &mut Bindings| pattern::each(env, module, params, b);
            let scoped = scoped(env, module, &context, each, body)?;
            let def = Def::Def {
                name: name.text.clone(),
                region: name.range,
                params: scoped.patterns,
                body: scoped.body,
            };
            Ok(Defined {
                def,
                annotation: None,
                free: scoped.free,
            })
        }
        Some(ty) => {
            let annotation = types::to_annotation(env, module, ty)?;
            let typed =
                |b: &mut Bindings| typed_args(env, module, &name.text, params, &annotation.ty, b);
            let scoped = scoped(env, module, &context, typed, body)?;
            let (params, result) = scoped.patterns;
            let def = Def::TypedDef {
                name: name.text.clone(),
                region: name.range,
                free: annotation.free.clone(),
                params,
                body: scoped.body,
                result,
            };
            Ok(Defined {
                def,
                annotation: Some(annotation),
                free: scoped.free,
            })
        }
    }
}

/// `gatherTypedArgs`: each parameter against the argument its annotation
/// gives it, stopping at the first that fails, or where the annotation
/// runs out of arguments.
fn typed_args(
    env: &Env<'_>,
    module: &hir::Module,
    name: &str,
    params: &[PatId],
    ty: &Type,
    bindings: &mut Bindings,
) -> R<(Vec<(Pattern, Type)>, Type)> {
    let mut ty = ty.clone();
    let mut typed = Vec::with_capacity(params.len());
    for (index, &param) in params.iter().enumerate() {
        if let Type::Lambda(arg, result) = ty.iterated_dealias() {
            let pattern = pattern::canonicalize(env, module, param, bindings)?;
            typed.push((pattern, *arg));
            ty = *result;
        } else {
            let range = |id| module.source_map.pattern_range(id);
            let first = range(param);
            let last = range(*params.last().expect("a parameter"));
            return throw(Error::AnnotationTooShort {
                range: TextRange::new(first.start(), last.end()),
                name: name.to_owned(),
                index,
                leftover: params.len() - index,
            });
        }
    }
    Ok((typed, ty))
}

/// `getPatternNames`, in the order it lists them. 0.19.1's parser keeps
/// a record pattern's fields last first.
fn pattern_names(compat: Compat, module: &hir::Module, id: PatId) -> Vec<Spanned> {
    // Upstream conses each name onto those before it, so they are
    // gathered last first and turned round at the end.
    fn go(compat: Compat, module: &hir::Module, id: PatId, names: &mut Vec<Spanned>) {
        grow(|| match &module.patterns[id] {
            hir::Pattern::Var(name) => {
                let range = module.source_map.pattern_range(id);
                let text = name.clone();
                names.push(Spanned { text, range });
            }
            hir::Pattern::Record(fields) if compat == Compat::V0_19_1 => {
                names.extend(fields.iter().cloned());
            }
            hir::Pattern::Record(fields) => names.extend(fields.iter().rev().cloned()),
            hir::Pattern::Alias(inner, name) => {
                names.push(name.clone());
                go(compat, module, *inner, names);
            }
            hir::Pattern::Tuple(items) | hir::Pattern::List(items) => {
                for &p in items {
                    go(compat, module, p, names);
                }
            }
            hir::Pattern::Ctor { args, .. } => {
                for &p in args {
                    go(compat, module, p, names);
                }
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
        });
    }
    let mut names = Vec::new();
    go(compat, module, id, &mut names);
    names.reverse();
    names
}
