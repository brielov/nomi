//! Types as type errors show them (`Type/Error.hs`): every variable
//! named, a function's arguments flat, and a type that contains itself
//! cut short.

use std::collections::BTreeMap;

use nomi_base::diagnostic::doc::{Color, Doc, text};

use super::in_name_order;
use super::localizer::Localizer;
use super::render::{self, Context};
use crate::canonical::Home;

/// `T.Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// The first argument, the second, then the rest and the result.
    Lambda(Box<Type>, Box<Type>, Vec<Type>),
    Infinite,
    Error,
    FlexVar(String),
    FlexSuper(Super, String),
    RigidVar(String),
    RigidSuper(Super, String),
    Type(Home, String, Vec<Type>),
    Record(BTreeMap<String, Type>, Extension),
    Unit,
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
    Alias(Home, String, Vec<(String, Type)>, Box<Type>),
}

/// `T.Super`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Super {
    Number,
    Comparable,
    Appendable,
    CompAppend,
}

/// `T.Extension`: what a record type extends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Extension {
    Closed,
    FlexOpen(String),
    RigidOpen(String),
}

/// `iteratedDealias`: what `ty` stands for under all its aliases.
pub fn iterated_dealias(ty: Type) -> Type {
    match ty {
        Type::Alias(_, _, _, real) => iterated_dealias(*real),
        ty => ty,
    }
}

/// `toDoc`.
pub fn to_doc(localizer: &Localizer, context: Context, ty: &Type) -> Doc {
    match ty {
        Type::Lambda(a, b, cs) => render::lambda(
            context,
            to_doc(localizer, Context::Func, a),
            to_doc(localizer, Context::Func, b),
            cs.iter()
                .map(|c| to_doc(localizer, Context::Func, c))
                .collect(),
        ),
        Type::Infinite => text("∞"),
        Type::Error => text("?"),
        Type::FlexVar(name)
        | Type::FlexSuper(_, name)
        | Type::RigidVar(name)
        | Type::RigidSuper(_, name) => text(name.clone()),
        Type::Type(home, name, args) => render::apply(
            context,
            localizer.to_doc(home, name),
            args.iter()
                .map(|a| to_doc(localizer, Context::App, a))
                .collect(),
        ),
        Type::Record(fields, ext) => {
            render::record(fields_to_docs(localizer, fields), ext_to_doc(ext))
        }
        Type::Unit => text("()"),
        Type::Tuple(a, b, c) => render::tuple(
            to_doc(localizer, Context::None, a),
            to_doc(localizer, Context::None, b),
            c.iter()
                .map(|c| to_doc(localizer, Context::None, c))
                .collect(),
        ),
        Type::Alias(home, name, args, _) => alias_to_doc(localizer, context, home, name, args),
    }
}

/// `aliasToDoc`.
fn alias_to_doc(
    localizer: &Localizer,
    context: Context,
    home: &Home,
    name: &str,
    args: &[(String, Type)],
) -> Doc {
    render::apply(
        context,
        localizer.to_doc(home, name),
        args.iter()
            .map(|(_, a)| to_doc(localizer, Context::App, a))
            .collect(),
    )
}

/// `fieldsToDocs`: by name, in the release's order.
fn fields_to_docs(localizer: &Localizer, fields: &BTreeMap<String, Type>) -> Vec<(Doc, Doc)> {
    (in_name_order(localizer.compat, fields).into_iter())
        .map(|(n, t)| (text(n.clone()), to_doc(localizer, Context::None, t)))
        .collect()
}

/// The names of `map`, in the release's order.
fn names_in_order<T>(compat: nomi_base::Compat, map: &BTreeMap<String, T>) -> Vec<String> {
    in_name_order(compat, map)
        .into_iter()
        .map(|(n, _)| n.clone())
        .collect()
}

/// `extToDoc`.
fn ext_to_doc(ext: &Extension) -> Option<Doc> {
    match ext {
        Extension::Closed => None,
        Extension::FlexOpen(x) | Extension::RigidOpen(x) => Some(text(x.clone())),
    }
}

/// `Problem`: what a comparison of two types found, to explain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    IntFloat,
    StringFromInt,
    StringFromFloat,
    StringToInt,
    StringToFloat,
    AnythingToBool,
    AnythingFromMaybe,
    ArityMismatch(usize, usize),
    BadFlexSuper(Direction, Super, String, Type),
    BadRigidVar(String, Type),
    BadRigidSuper(Super, String, Type),
    FieldTypo(String, Vec<String>),
    FieldsMissing(Vec<String>),
}

/// `Direction`: which side of a comparison a super type is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Have,
    Need,
}

/// `Status`: alike, or different with what explains it.
#[derive(Clone, Debug)]
enum Status {
    Similar,
    Different(Vec<Problem>),
}

/// `merge`.
fn merge(a: Status, b: Status) -> Status {
    match (a, b) {
        (Status::Similar, b) => b,
        (a, Status::Similar) => a,
        (Status::Different(mut p1), Status::Different(p2)) => {
            p1.extend(p2);
            Status::Different(p1)
        }
    }
}

/// `Diff`: the two types' docs, each with the parts that differ marked,
/// and how they compare.
struct Diff<T> {
    a: T,
    b: T,
    status: Status,
}

impl<T> Diff<T> {
    fn map<U>(self, f: impl Fn(T) -> U) -> Diff<U> {
        Diff {
            a: f(self.a),
            b: f(self.b),
            status: self.status,
        }
    }
}

/// `sequenceA`: the diffs' docs side by side, their statuses merged in
/// order.
fn sequence<T>(diffs: Vec<Diff<T>>) -> Diff<Vec<T>> {
    let mut a = Vec::with_capacity(diffs.len());
    let mut b = Vec::with_capacity(diffs.len());
    let mut status = Status::Similar;
    for d in diffs {
        a.push(d.a);
        b.push(d.b);
        status = merge(status, d.status);
    }
    Diff { a, b, status }
}

/// `toComparison`: both types' docs, the differences marked, and the
/// problems found.
pub fn to_comparison(localizer: &Localizer, t1: &Type, t2: &Type) -> (Doc, Doc, Vec<Problem>) {
    let diff = to_diff(localizer, Context::None, t1, t2);
    let problems = match diff.status {
        Status::Similar => Vec::new(),
        Status::Different(problems) => problems,
    };
    (diff.a, diff.b, problems)
}

fn same(localizer: &Localizer, context: Context, ty: &Type) -> Diff<Doc> {
    let doc = to_doc(localizer, context, ty);
    Diff {
        a: doc.clone(),
        b: doc,
        status: Status::Similar,
    }
}

fn similar(localizer: &Localizer, context: Context, t1: &Type, t2: &Type) -> Diff<Doc> {
    Diff {
        a: to_doc(localizer, context, t1),
        b: to_doc(localizer, context, t2),
        status: Status::Similar,
    }
}

fn different(a: Doc, b: Doc, problems: Vec<Problem>) -> Diff<Doc> {
    Diff {
        a,
        b,
        status: Status::Different(problems),
    }
}

fn yellow(doc: Doc) -> Doc {
    doc.color(Color::DullYellow)
}

fn home_is(home: &Home, module: &str) -> bool {
    home.package == "elm/core" && home.module == module
}

pub fn is_int(home: &Home, name: &str) -> bool {
    home_is(home, "Basics") && name == "Int"
}

pub fn is_float(home: &Home, name: &str) -> bool {
    home_is(home, "Basics") && name == "Float"
}

pub fn is_string(home: &Home, name: &str) -> bool {
    home_is(home, "String") && name == "String"
}

pub fn is_char(home: &Home, name: &str) -> bool {
    home_is(home, "Char") && name == "Char"
}

pub fn is_list(home: &Home, name: &str) -> bool {
    home_is(home, "List") && name == "List"
}

fn is_bool(home: &Home, name: &str) -> bool {
    home_is(home, "Basics") && name == "Bool"
}

fn is_maybe(home: &Home, name: &str) -> bool {
    home_is(home, "Maybe") && name == "Maybe"
}

/// `isSuper`.
fn is_super(super_type: Super, ty: &Type) -> bool {
    match iterated_dealias(ty.clone()) {
        Type::Type(h, n, args) => match super_type {
            Super::Number => is_int(&h, &n) || is_float(&h, &n),
            Super::Comparable => {
                is_int(&h, &n)
                    || is_float(&h, &n)
                    || is_string(&h, &n)
                    || is_char(&h, &n)
                    || (is_list(&h, &n) && args.first().is_some_and(|a| is_super(super_type, a)))
            }
            Super::Appendable => is_string(&h, &n) || is_list(&h, &n),
            Super::CompAppend => {
                is_string(&h, &n)
                    || (is_list(&h, &n)
                        && args.first().is_some_and(|a| is_super(Super::Comparable, a)))
            }
        },
        Type::Tuple(a, b, c) => match super_type {
            Super::Comparable => {
                is_super(super_type, &a)
                    && is_super(super_type, &b)
                    && c.is_none_or(|c| is_super(super_type, &c))
            }
            Super::Number | Super::Appendable | Super::CompAppend => false,
        },
        _ => false,
    }
}

/// `nameClashToDoc`: a type of the same name as another, by its module.
fn name_clash_to_doc(
    localizer: &Localizer,
    context: Context,
    home: &Home,
    name: &str,
    args: &[Type],
) -> Doc {
    let name = text(home.module.clone())
        .color(Color::Yellow)
        .append(yellow(text(format!(".{name}"))));
    render::apply(
        context,
        name,
        args.iter()
            .map(|a| to_doc(localizer, Context::App, a))
            .collect(),
    )
}

/// `toDiff`.
fn to_diff(localizer: &Localizer, ctx: Context, t1: &Type, t2: &Type) -> Diff<Doc> {
    use Type as T;
    match (t1, t2) {
        (T::Unit, T::Unit) | (T::Error, T::Error) | (T::Infinite, T::Infinite) => {
            return same(localizer, ctx, t1);
        }
        (T::FlexVar(x), T::FlexVar(y))
        | (T::FlexSuper(_, x), T::FlexSuper(_, y))
        | (T::RigidVar(x), T::RigidVar(y))
        | (T::RigidSuper(_, x), T::RigidSuper(_, y))
            if x == y =>
        {
            return same(localizer, ctx, t1);
        }
        (T::FlexVar(_), _) | (_, T::FlexVar(_)) => return similar(localizer, ctx, t1, t2),
        (T::FlexSuper(s, _), t) | (t, T::FlexSuper(s, _)) if is_super(*s, t) => {
            return similar(localizer, ctx, t1, t2);
        }
        _ => {}
    }
    match (t1, t2) {
        (T::Lambda(a, b, cs), T::Lambda(x, y, zs)) => {
            if cs.len() == zs.len() {
                let first = to_diff(localizer, Context::Func, a, x);
                let second = to_diff(localizer, Context::Func, b, y);
                let rest = sequence(
                    cs.iter()
                        .zip(zs)
                        .map(|(c, z)| to_diff(localizer, Context::Func, c, z))
                        .collect(),
                );
                let status = merge(merge(first.status, second.status), rest.status);
                Diff {
                    a: render::lambda(ctx, first.a, second.a, rest.a),
                    b: render::lambda(ctx, first.b, second.b, rest.b),
                    status,
                }
            } else {
                let f = |t: &Type| to_doc(localizer, Context::Func, t);
                different(
                    yellow(render::lambda(ctx, f(a), f(b), cs.iter().map(f).collect())),
                    yellow(render::lambda(ctx, f(x), f(y), zs.iter().map(f).collect())),
                    vec![Problem::ArityMismatch(2 + cs.len(), 2 + zs.len())],
                )
            }
        }
        (T::Tuple(a, b, None), T::Tuple(x, y, None)) => {
            let first = to_diff(localizer, Context::None, a, x);
            let second = to_diff(localizer, Context::None, b, y);
            Diff {
                a: render::tuple(first.a, second.a, Vec::new()),
                b: render::tuple(first.b, second.b, Vec::new()),
                status: merge(first.status, second.status),
            }
        }
        (T::Tuple(a, b, Some(c)), T::Tuple(x, y, Some(z))) => {
            let first = to_diff(localizer, Context::None, a, x);
            let second = to_diff(localizer, Context::None, b, y);
            let third = to_diff(localizer, Context::None, c, z);
            Diff {
                a: render::tuple(first.a, second.a, vec![third.a]),
                b: render::tuple(first.b, second.b, vec![third.b]),
                status: merge(merge(first.status, second.status), third.status),
            }
        }
        (T::Record(fields1, ext1), T::Record(fields2, ext2)) => {
            diff_record(localizer, fields1, ext1, fields2, ext2)
        }
        (T::Type(home1, name1, args1), T::Type(home2, name2, args2))
            if home1 == home2 && name1 == name2 =>
        {
            let args = sequence(
                args1
                    .iter()
                    .zip(args2)
                    .map(|(a, b)| to_diff(localizer, Context::App, a, b))
                    .collect(),
            );
            Diff {
                a: render::apply(ctx, localizer.to_doc(home1, name1), args.a),
                b: render::apply(ctx, localizer.to_doc(home1, name1), args.b),
                status: args.status,
            }
        }
        (T::Alias(home1, name1, args1, _), T::Alias(home2, name2, args2, _))
            if home1 == home2 && name1 == name2 =>
        {
            let args = sequence(
                (args1.iter().zip(args2))
                    .map(|((_, a), (_, b))| to_diff(localizer, Context::App, a, b))
                    .collect(),
            );
            Diff {
                a: render::apply(ctx, localizer.to_doc(home1, name1), args.a),
                b: render::apply(ctx, localizer.to_doc(home1, name1), args.b),
                status: args.status,
            }
        }
        // Then, specific problems.
        (T::Type(home1, name1, args1), T::Type(home2, name2, args2))
            if localizer.to_chars(home1, name1) == localizer.to_chars(home2, name2) =>
        {
            different(
                name_clash_to_doc(localizer, ctx, home1, name1, args1),
                name_clash_to_doc(localizer, ctx, home2, name2, args2),
                Vec::new(),
            )
        }
        (T::Type(home, name, args), t2)
            if args.len() == 1
                && is_maybe(home, name)
                && is_similar(&to_diff(localizer, ctx, &args[0], t2)) =>
        {
            different(
                render::apply(
                    ctx,
                    yellow(localizer.to_doc(home, name)),
                    vec![to_doc(localizer, Context::App, &args[0])],
                ),
                to_doc(localizer, ctx, t2),
                vec![Problem::AnythingFromMaybe],
            )
        }
        (t1, T::Type(home, name, args))
            if args.len() == 1
                && is_list(home, name)
                && is_similar(&to_diff(localizer, ctx, t1, &args[0])) =>
        {
            different(
                to_doc(localizer, ctx, t1),
                render::apply(
                    ctx,
                    yellow(localizer.to_doc(home, name)),
                    vec![to_doc(localizer, Context::App, &args[0])],
                ),
                Vec::new(),
            )
        }
        (T::Alias(home1, name1, args1, real1), t2) => {
            match diff_aliased_record(localizer, real1, t2) {
                Some(diff) => Diff {
                    a: yellow(alias_to_doc(localizer, ctx, home1, name1, args1)),
                    b: diff.b,
                    status: diff.status,
                },
                None => match t2 {
                    T::Type(home2, name2, args2)
                        if localizer.to_chars(home1, name1) == localizer.to_chars(home2, name2) =>
                    {
                        let args1: Vec<Type> = args1.iter().map(|(_, a)| a.clone()).collect();
                        different(
                            name_clash_to_doc(localizer, ctx, home1, name1, &args1),
                            name_clash_to_doc(localizer, ctx, home2, name2, args2),
                            Vec::new(),
                        )
                    }
                    _ => different(
                        yellow(to_doc(localizer, ctx, t1)),
                        yellow(to_doc(localizer, ctx, t2)),
                        Vec::new(),
                    ),
                },
            }
        }
        (t1, T::Alias(home2, name2, args2, real2)) => {
            match diff_aliased_record(localizer, t1, real2) {
                Some(diff) => Diff {
                    a: diff.a,
                    b: yellow(alias_to_doc(localizer, ctx, home2, name2, args2)),
                    status: diff.status,
                },
                None => match t1 {
                    T::Type(home1, name1, args1)
                        if localizer.to_chars(home1, name1) == localizer.to_chars(home2, name2) =>
                    {
                        let args2: Vec<Type> = args2.iter().map(|(_, a)| a.clone()).collect();
                        different(
                            name_clash_to_doc(localizer, ctx, home1, name1, args1),
                            name_clash_to_doc(localizer, ctx, home2, name2, &args2),
                            Vec::new(),
                        )
                    }
                    _ => different(
                        yellow(to_doc(localizer, ctx, t1)),
                        yellow(to_doc(localizer, ctx, t2)),
                        Vec::new(),
                    ),
                },
            }
        }
        (t1, t2) => {
            let doc1 = yellow(to_doc(localizer, ctx, t1));
            let doc2 = yellow(to_doc(localizer, ctx, t2));
            let problems = match (t1, t2) {
                (T::RigidVar(x), other) => vec![Problem::BadRigidVar(x.clone(), other.clone())],
                (T::FlexSuper(s, x), other) => vec![Problem::BadFlexSuper(
                    Direction::Have,
                    *s,
                    x.clone(),
                    other.clone(),
                )],
                (T::RigidSuper(s, x), other) => {
                    vec![Problem::BadRigidSuper(*s, x.clone(), other.clone())]
                }
                (other, T::RigidVar(x)) => vec![Problem::BadRigidVar(x.clone(), other.clone())],
                (other, T::FlexSuper(s, x)) => vec![Problem::BadFlexSuper(
                    Direction::Need,
                    *s,
                    x.clone(),
                    other.clone(),
                )],
                (other, T::RigidSuper(s, x)) => {
                    vec![Problem::BadRigidSuper(*s, x.clone(), other.clone())]
                }
                (T::Type(h1, n1, a1), T::Type(h2, n2, a2)) if a1.is_empty() && a2.is_empty() => {
                    if is_int(h1, n1) && is_float(h2, n2) || is_float(h1, n1) && is_int(h2, n2) {
                        vec![Problem::IntFloat]
                    } else if is_int(h1, n1) && is_string(h2, n2) {
                        vec![Problem::StringFromInt]
                    } else if is_float(h1, n1) && is_string(h2, n2) {
                        vec![Problem::StringFromFloat]
                    } else if is_string(h1, n1) && is_int(h2, n2) {
                        vec![Problem::StringToInt]
                    } else if is_string(h1, n1) && is_float(h2, n2) {
                        vec![Problem::StringToFloat]
                    } else if is_bool(h2, n2) {
                        vec![Problem::AnythingToBool]
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            };
            different(doc1, doc2, problems)
        }
    }
}

fn is_similar<T>(diff: &Diff<T>) -> bool {
    matches!(diff.status, Status::Similar)
}

/// `diffAliasedRecord`.
fn diff_aliased_record(localizer: &Localizer, t1: &Type, t2: &Type) -> Option<Diff<Doc>> {
    match (iterated_dealias(t1.clone()), iterated_dealias(t2.clone())) {
        (Type::Record(fields1, ext1), Type::Record(fields2, ext2)) => {
            Some(diff_record(localizer, &fields1, &ext1, &fields2, &ext2))
        }
        _ => None,
    }
}

/// `diffRecord`: fields only one side has marked, the shared ones
/// compared, and what the extensions allow found as a typo or as missing
/// fields.
fn diff_record(
    localizer: &Localizer,
    fields1: &BTreeMap<String, Type>,
    ext1: &Extension,
    fields2: &BTreeMap<String, Type>,
    ext2: &Extension,
) -> Diff<Doc> {
    let unknown = |field: &str, ty: &Type| {
        (
            yellow(text(field.to_owned())),
            to_doc(localizer, Context::None, ty),
        )
    };
    let left: BTreeMap<&String, (Doc, Doc)> = (fields1.iter())
        .filter(|(n, _)| !fields2.contains_key(*n))
        .map(|(n, t)| (n, unknown(n, t)))
        .collect();
    let right: BTreeMap<&String, (Doc, Doc)> = (fields2.iter())
        .filter(|(n, _)| !fields1.contains_key(*n))
        .map(|(n, t)| (n, unknown(n, t)))
        .collect();
    let both: Vec<(&String, Diff<(Doc, Doc)>)> = (fields1.iter())
        .filter_map(|(n, t1)| {
            let t2 = fields2.get(n)?;
            let diff = to_diff(localizer, Context::None, t1, t2);
            Some((n, diff.map(|d| (text(n.clone()), d))))
        })
        .collect();
    let names: Vec<&String> = both.iter().map(|(n, _)| *n).collect();
    let both = sequence(both.into_iter().map(|(_, d)| d).collect());
    // `Map.elems (Map.union both left)`, and the right side alike: by name,
    // in the release's order.
    let order = crate::canonicalize::name_order(localizer.compat);
    let mut docs1: Vec<(&String, (Doc, Doc))> = names.iter().copied().zip(both.a).collect();
    let mut docs2: Vec<(&String, (Doc, Doc))> = names.iter().copied().zip(both.b).collect();
    let fields_status = if left.is_empty() && right.is_empty() {
        both.status
    } else {
        merge(both.status, Status::Different(Vec::new()))
    };
    let mut left_names: Vec<String> = left.keys().map(|n| (*n).clone()).collect();
    let mut right_names: Vec<String> = right.keys().map(|n| (*n).clone()).collect();
    left_names.sort_by(|a, b| order(a, b));
    right_names.sort_by(|a, b| order(a, b));
    docs1.extend(left);
    docs2.extend(right);
    docs1.sort_by(|a, b| order(a.0, b.0));
    docs2.sort_by(|a, b| order(a.0, b.0));
    let ext = ext_to_diff(ext1, ext2);
    let status = merge(fields_status, ext.status);
    let a = render::record(docs1.into_iter().map(|(_, d)| d).collect(), ext.a);
    let b = render::record(docs2.into_iter().map(|(_, d)| d).collect(), ext.b);
    let fixed = |e: &Extension| !matches!(e, Extension::FlexOpen(_));
    let extra = match (fixed(ext1), fixed(ext2)) {
        (true, true) => match left_names.first() {
            Some(f) => Status::Different(vec![Problem::FieldTypo(
                f.clone(),
                names_in_order(localizer.compat, fields2),
            )]),
            None if right_names.is_empty() => Status::Similar,
            None => Status::Different(vec![Problem::FieldsMissing(right_names)]),
        },
        (false, true) => match left_names.first() {
            Some(f) => Status::Different(vec![Problem::FieldTypo(
                f.clone(),
                names_in_order(localizer.compat, fields2),
            )]),
            None => Status::Similar,
        },
        (true, false) => match right_names.first() {
            Some(f) => Status::Different(vec![Problem::FieldTypo(
                f.clone(),
                names_in_order(localizer.compat, fields1),
            )]),
            None => Status::Similar,
        },
        (false, false) => Status::Similar,
    };
    Diff {
        a,
        b,
        status: merge(status, extra),
    }
}

/// `extToDiff`.
fn ext_to_diff(ext1: &Extension, ext2: &Extension) -> Diff<Option<Doc>> {
    let status = match (ext1, ext2) {
        (Extension::Closed, Extension::Closed | Extension::FlexOpen(_)) => Status::Similar,
        (Extension::Closed, Extension::RigidOpen(_)) => Status::Different(Vec::new()),
        (Extension::FlexOpen(_), _) => Status::Similar,
        (Extension::RigidOpen(_), Extension::Closed) => Status::Different(Vec::new()),
        (Extension::RigidOpen(_), Extension::FlexOpen(_)) => Status::Similar,
        (Extension::RigidOpen(x), Extension::RigidOpen(y)) => {
            if x == y {
                Status::Similar
            } else {
                Status::Different(vec![Problem::BadRigidVar(
                    x.clone(),
                    Type::RigidVar(y.clone()),
                )])
            }
        }
    };
    let (a, b) = (ext_to_doc(ext1), ext_to_doc(ext2));
    match status {
        Status::Similar => Diff { a, b, status },
        Status::Different(_) => Diff {
            a: a.map(yellow),
            b: b.map(yellow),
            status,
        },
    }
}
