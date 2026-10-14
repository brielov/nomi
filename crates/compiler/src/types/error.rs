//! Types as type errors show them (`Type/Error.hs`): every variable
//! named, a function's arguments flat, and a type that contains itself
//! cut short.

use std::collections::BTreeMap;

use nomi_base::diagnostic::doc::{Color, Doc, dull_yellow as yellow, text};

use super::localizer::Localizer;
use super::render::{self, Context};
use super::{
    SuperType, in_name_order, is_bool, is_char, is_float, is_int, is_list, is_maybe, is_string,
};
use crate::canonical::Home;

/// `T.Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// The first argument, the second, then the rest and the result.
    Lambda(Box<Type>, Box<Type>, Vec<Type>),
    Infinite,
    Error,
    FlexVar(String),
    FlexSuper(SuperType, String),
    RigidVar(String),
    RigidSuper(SuperType, String),
    Type(Home, String, Vec<Type>),
    Record(BTreeMap<String, Type>, Extension),
    Unit,
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
    Alias(Home, String, Vec<(String, Type)>, Box<Type>),
}

/// `T.Extension`: what a record type extends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Extension {
    Closed,
    FlexOpen(String),
    RigidOpen(String),
}

/// `iteratedDealias`: what `ty` stands for under all its aliases.
pub(super) fn iterated_dealias(mut ty: &Type) -> &Type {
    while let Type::Alias(_, _, _, real) = ty {
        ty = real;
    }
    ty
}

/// `toDoc`.
pub(super) fn to_doc(localizer: &Localizer, context: Context, ty: &Type) -> Doc {
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
pub(super) fn names_in_order<T>(
    compat: nomi_base::Compat,
    map: &BTreeMap<String, T>,
) -> Vec<String> {
    in_name_order(compat, map)
        .into_iter()
        .map(|(n, _)| n.clone())
        .collect()
}

/// `extToDoc`.
pub(super) fn ext_to_doc(ext: &Extension) -> Option<Doc> {
    match ext {
        Extension::Closed => None,
        Extension::FlexOpen(x) | Extension::RigidOpen(x) => Some(text(x.clone())),
    }
}

/// `Problem`: what a comparison of two types found, to explain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    IntFloat,
    StringFromInt,
    StringFromFloat,
    StringToInt,
    StringToFloat,
    AnythingToBool,
    AnythingFromMaybe,
    /// Functions of different arities: how many arguments each takes.
    ArityMismatch {
        actual: usize,
        expected: usize,
    },
    /// A constrained variable on `direction`'s side, where `other` is.
    BadFlexSuper {
        direction: Direction,
        super_type: SuperType,
        other: Type,
    },
    BadRigidVar(String, Type),
    BadRigidSuper(SuperType, String, Type),
    FieldTypo(String, Vec<String>),
    FieldsMissing(Vec<String>),
}

/// `Direction`: which side of a comparison a super type is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Have,
    Need,
}

impl Direction {
    /// `x` and `y` in comparison order, `x` on this side.
    fn order<T>(self, x: T, y: T) -> (T, T) {
        match self {
            Direction::Have => (x, y),
            Direction::Need => (y, x),
        }
    }
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

/// `toComparison`'s result: both types' docs, the differences marked,
/// and the problems found.
pub(super) struct Comparison {
    pub(super) actual: Doc,
    pub(super) expected: Doc,
    pub(super) problems: Vec<Problem>,
}

/// `toComparison`.
pub(super) fn to_comparison(localizer: &Localizer, actual: &Type, expected: &Type) -> Comparison {
    let diff = to_diff(localizer, Context::None, actual, expected);
    let problems = match diff.status {
        Status::Similar => Vec::new(),
        Status::Different(problems) => problems,
    };
    Comparison {
        actual: diff.a,
        expected: diff.b,
        problems,
    }
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

/// `isSuper`.
fn is_super(super_type: SuperType, ty: &Type) -> bool {
    use SuperType::{Appendable, CompAppend, Comparable, Number};
    match iterated_dealias(ty) {
        Type::Type(h, n, args) => match super_type {
            Number => is_int(h, n) || is_float(h, n),
            Comparable => {
                is_int(h, n)
                    || is_float(h, n)
                    || is_string(h, n)
                    || is_char(h, n)
                    || (is_list(h, n) && args.first().is_some_and(|a| is_super(super_type, a)))
            }
            Appendable => is_string(h, n) || is_list(h, n),
            CompAppend => {
                is_string(h, n)
                    || (is_list(h, n) && args.first().is_some_and(|a| is_super(Comparable, a)))
            }
        },
        Type::Tuple(a, b, c) => match super_type {
            Comparable => {
                is_super(super_type, a)
                    && is_super(super_type, b)
                    && c.as_deref().is_none_or(|c| is_super(super_type, c))
            }
            Number | Appendable | CompAppend => false,
        },
        Type::Lambda(..)
        | Type::Infinite
        | Type::Error
        | Type::FlexVar(_)
        | Type::FlexSuper(..)
        | Type::RigidVar(_)
        | Type::RigidSuper(..)
        | Type::Record(..)
        | Type::Unit
        | Type::Alias(..) => false,
    }
}

/// `nameClashToDoc`: a type of the same name as another, by its module.
fn name_clash_to_doc<'t>(
    localizer: &Localizer,
    context: Context,
    home: &Home,
    name: &str,
    args: impl IntoIterator<Item = &'t Type>,
) -> Doc {
    let name = text(home.module.clone())
        .color(Color::Yellow)
        .append(yellow(text(format!(".{name}"))));
    render::apply(
        context,
        name,
        (args.into_iter())
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
                let rest = diffs(localizer, Context::Func, cs.iter().zip(zs));
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
                    vec![Problem::ArityMismatch {
                        actual: 2 + cs.len(),
                        expected: 2 + zs.len(),
                    }],
                )
            }
        }
        (T::Tuple(a, b, c), T::Tuple(x, y, z)) if c.is_some() == z.is_some() => {
            let first = to_diff(localizer, Context::None, a, x);
            let second = to_diff(localizer, Context::None, b, y);
            let rest = diffs(
                localizer,
                Context::None,
                c.iter().zip(z).map(|(c, z)| (&**c, &**z)),
            );
            Diff {
                a: render::tuple(first.a, second.a, rest.a),
                b: render::tuple(first.b, second.b, rest.b),
                status: merge(merge(first.status, second.status), rest.status),
            }
        }
        (T::Record(fields1, ext1), T::Record(fields2, ext2)) => {
            diff_record(localizer, fields1, ext1, fields2, ext2)
        }
        (T::Type(home1, name1, args1), T::Type(home2, name2, args2))
            if home1 == home2 && name1 == name2 =>
        {
            diff_args(localizer, ctx, home1, name1, args1.iter().zip(args2))
        }
        (T::Alias(home1, name1, args1, _), T::Alias(home2, name2, args2, _))
            if home1 == home2 && name1 == name2 =>
        {
            let pairs = args1.iter().zip(args2).map(|((_, a), (_, b))| (a, b));
            diff_args(localizer, ctx, home1, name1, pairs)
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
        (T::Alias(..), _) => diff_alias(localizer, ctx, t1, t2, Direction::Have),
        (_, T::Alias(..)) => diff_alias(localizer, ctx, t2, t1, Direction::Need),
        (t1, t2) => different(
            yellow(to_doc(localizer, ctx, t1)),
            yellow(to_doc(localizer, ctx, t2)),
            problems(t1, t2),
        ),
    }
}

/// `sequenceA` of each pair's diff in `context`.
fn diffs<'t>(
    localizer: &Localizer,
    context: Context,
    pairs: impl Iterator<Item = (&'t Type, &'t Type)>,
) -> Diff<Vec<Doc>> {
    sequence(
        pairs
            .map(|(a, b)| to_diff(localizer, context, a, b))
            .collect(),
    )
}

/// Two types of one name, `name` of `home`, applied to the arguments
/// `pairs` compares.
fn diff_args<'t>(
    localizer: &Localizer,
    ctx: Context,
    home: &Home,
    name: &str,
    pairs: impl Iterator<Item = (&'t Type, &'t Type)>,
) -> Diff<Doc> {
    let args = diffs(localizer, Context::App, pairs);
    Diff {
        a: render::apply(ctx, localizer.to_doc(home, name), args.a),
        b: render::apply(ctx, localizer.to_doc(home, name), args.b),
        status: args.status,
    }
}

/// `toDiff`'s alias on one side, `side`, and `other` on the other: the
/// records they stand for compared, or a clash of names, or just
/// different.
fn diff_alias(
    localizer: &Localizer,
    ctx: Context,
    alias: &Type,
    other: &Type,
    side: Direction,
) -> Diff<Doc> {
    let Type::Alias(home, name, args, real) = alias else {
        unreachable!("an alias");
    };
    let (t1, t2) = side.order(&**real, other);
    if let Some(diff) = diff_aliased_record(localizer, t1, t2) {
        let alias = yellow(alias_to_doc(localizer, ctx, home, name, args));
        let (a, b) = match side {
            Direction::Have => (alias, diff.b),
            Direction::Need => (diff.a, alias),
        };
        return Diff {
            a,
            b,
            status: diff.status,
        };
    }
    let (a, b) = match other {
        Type::Type(other_home, other_name, other_args)
            if localizer.to_chars(home, name) == localizer.to_chars(other_home, other_name) =>
        {
            let alias_args = args.iter().map(|(_, a)| a);
            side.order(
                name_clash_to_doc(localizer, ctx, home, name, alias_args),
                name_clash_to_doc(localizer, ctx, other_home, other_name, other_args),
            )
        }
        _ => side.order(
            yellow(to_doc(localizer, ctx, alias)),
            yellow(to_doc(localizer, ctx, other)),
        ),
    };
    different(a, b, Vec::new())
}

/// What explains two types that differ outright.
fn problems(t1: &Type, t2: &Type) -> Vec<Problem> {
    use Type as T;
    let bad_flex_super = |direction, super_type, other: &Type| Problem::BadFlexSuper {
        direction,
        super_type,
        other: other.clone(),
    };
    let problem = match (t1, t2) {
        (T::RigidVar(x), other) => Problem::BadRigidVar(x.clone(), other.clone()),
        (T::FlexSuper(s, _), other) => bad_flex_super(Direction::Have, *s, other),
        (T::RigidSuper(s, x), other) => Problem::BadRigidSuper(*s, x.clone(), other.clone()),
        (other, T::RigidVar(x)) => Problem::BadRigidVar(x.clone(), other.clone()),
        (other, T::FlexSuper(s, _)) => bad_flex_super(Direction::Need, *s, other),
        (other, T::RigidSuper(s, x)) => Problem::BadRigidSuper(*s, x.clone(), other.clone()),
        (T::Type(h1, n1, a1), T::Type(h2, n2, a2)) if a1.is_empty() && a2.is_empty() => {
            if is_int(h1, n1) && is_float(h2, n2) || is_float(h1, n1) && is_int(h2, n2) {
                Problem::IntFloat
            } else if is_int(h1, n1) && is_string(h2, n2) {
                Problem::StringFromInt
            } else if is_float(h1, n1) && is_string(h2, n2) {
                Problem::StringFromFloat
            } else if is_string(h1, n1) && is_int(h2, n2) {
                Problem::StringToInt
            } else if is_string(h1, n1) && is_float(h2, n2) {
                Problem::StringToFloat
            } else if is_bool(h2, n2) {
                Problem::AnythingToBool
            } else {
                return Vec::new();
            }
        }
        _ => return Vec::new(),
    };
    vec![problem]
}

fn is_similar<T>(diff: &Diff<T>) -> bool {
    matches!(diff.status, Status::Similar)
}

/// `diffAliasedRecord`.
fn diff_aliased_record(localizer: &Localizer, t1: &Type, t2: &Type) -> Option<Diff<Doc>> {
    match (iterated_dealias(t1), iterated_dealias(t2)) {
        (Type::Record(fields1, ext1), Type::Record(fields2, ext2)) => {
            Some(diff_record(localizer, fields1, ext1, fields2, ext2))
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
    let compat = localizer.compat;
    let mut docs1: Vec<(&String, (Doc, Doc))> = names.iter().copied().zip(both.a).collect();
    let mut docs2: Vec<(&String, (Doc, Doc))> = names.iter().copied().zip(both.b).collect();
    let fields_status = if left.is_empty() && right.is_empty() {
        both.status
    } else {
        merge(both.status, Status::Different(Vec::new()))
    };
    let left_names = compat.sorted_names(left.keys().map(|n| (*n).clone()));
    let right_names = compat.sorted_names(right.keys().map(|n| (*n).clone()));
    docs1.extend(left);
    docs2.extend(right);
    docs1.sort_by(|a, b| compat.compare_names(a.0, b.0));
    docs2.sort_by(|a, b| compat.compare_names(a.0, b.0));
    let ext = ext_to_diff(ext1, ext2);
    let status = merge(fields_status, ext.status);
    let a = render::record(docs1.into_iter().map(|(_, d)| d).collect(), ext.a);
    let b = render::record(docs2.into_iter().map(|(_, d)| d).collect(), ext.b);
    let fixed = |e: &Extension| !matches!(e, Extension::FlexOpen(_));
    let typo = |field: &String, fields: &BTreeMap<String, Type>| {
        let names = names_in_order(compat, fields);
        Status::Different(vec![Problem::FieldTypo(field.clone(), names)])
    };
    let extra = match (fixed(ext1), fixed(ext2)) {
        (true, true) => match left_names.first() {
            Some(f) => typo(f, fields2),
            None if right_names.is_empty() => Status::Similar,
            None => Status::Different(vec![Problem::FieldsMissing(right_names)]),
        },
        (false, true) => left_names
            .first()
            .map_or(Status::Similar, |f| typo(f, fields2)),
        (true, false) => right_names
            .first()
            .map_or(Status::Similar, |f| typo(f, fields1)),
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
