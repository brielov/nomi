//! `elm diff` (`Diff.hs`) and the API diff it and `elm bump` compute
//! (`Deps/Diff.hs`): two releases' documentation compared module by
//! module, each value, union, alias and operator added, removed or
//! changed, a change ignoring how type variables are named, and the
//! magnitude of the whole. Each list is in the release's order of names.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use nomi_base::Compat;
use nomi_base::diagnostic::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, hsep, reflow, sep, text, vcat, words};
use nomi_compiler::hir::Associativity;
use nomi_compiler::types::{SuperType, render};
use nomi_project::report::elm_json_path;
use nomi_project::version::Version;
use nomi_project::{Name, Outline};
use nomi_registry::{Http, HttpError, Index};
use nomi_syntax::ast::{self, AstNode};
use serde_json::Value as Json;

use crate::check::{self, Build, Checked, Goal, env_of, foreigns};
use crate::console::{self, Console, Personality};
use crate::paths::Root;
use crate::project::{Project, check_source_dirs};
use crate::report::one_line;
use crate::{Context, env};

/// `Elm.Compiler.Type.Type`: a type as documentation writes it, its names
/// unqualified as the decoder reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::enum_variant_names, reason = "upstream's `Type.Type`")]
enum Type {
    Lambda(Box<Type>, Box<Type>),
    Var(String),
    Type(String, Vec<Type>),
    Record(Vec<(String, Type)>, Option<String>),
    Unit,
    Tuple(Box<Type>, Box<Type>, Vec<Type>),
}

/// `Docs.Union`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Union {
    vars: Vec<String>,
    ctors: Vec<(String, Vec<Type>)>,
}

/// `Docs.Alias`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Alias {
    vars: Vec<String>,
    ty: Type,
}

/// `Docs.Binop`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binop {
    ty: Type,
    associativity: Associativity,
    precedence: i64,
}

/// `Docs.Module`, each kind of entry by name in the release's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Module {
    unions: Vec<(String, Union)>,
    aliases: Vec<(String, Alias)>,
    values: Vec<(String, Type)>,
    binops: Vec<(String, Binop)>,
}

/// `Docs.Documentation`: the modules by name in the release's order.
pub(crate) type Documentation = Vec<(String, Module)>;

/// `Docs.decoder`: `docs.json` read, its types parsed; `None` for
/// anything else.
fn decode(compat: Compat, bytes: &[u8]) -> Option<Documentation> {
    let json: Json = serde_json::from_slice(bytes).ok()?;
    let mut modules = Vec::new();
    for module in json.as_array()? {
        let name = module["name"].as_str()?.to_owned();
        let strings = |value: &Json| -> Option<Vec<String>> {
            value
                .as_array()?
                .iter()
                .map(|s| s.as_str().map(str::to_owned))
                .collect()
        };
        let ty = |value: &Json| parse_type(value.as_str()?);
        let mut unions = Vec::new();
        for union in module["unions"].as_array()? {
            let mut ctors = Vec::new();
            for case in union["cases"].as_array()? {
                let case = case.as_array()?;
                let args: Option<Vec<Type>> = case.get(1)?.as_array()?.iter().map(ty).collect();
                ctors.push((case.first()?.as_str()?.to_owned(), args?));
            }
            let vars = strings(&union["args"])?;
            unions.push((union["name"].as_str()?.to_owned(), Union { vars, ctors }));
        }
        let mut aliases = Vec::new();
        for alias in module["aliases"].as_array()? {
            let entry = Alias {
                vars: strings(&alias["args"])?,
                ty: ty(&alias["type"])?,
            };
            aliases.push((alias["name"].as_str()?.to_owned(), entry));
        }
        let mut values = Vec::new();
        for value in module["values"].as_array()? {
            values.push((value["name"].as_str()?.to_owned(), ty(&value["type"])?));
        }
        let mut binops = Vec::new();
        for binop in module["binops"].as_array()? {
            let entry = Binop {
                ty: ty(&binop["type"])?,
                associativity: Associativity::from_json_name(binop["associativity"].as_str()?)?,
                precedence: binop["precedence"].as_i64()?,
            };
            binops.push((binop["name"].as_str()?.to_owned(), entry));
        }
        let module = Module {
            unions: keyed(compat, unions),
            aliases: keyed(compat, aliases),
            values: keyed(compat, values),
            binops: keyed(compat, binops),
        };
        modules.push((name, module));
    }
    Some(keyed(compat, modules))
}

/// `Map.fromList`: by name in the release's order, the last of a name
/// kept.
fn keyed<T>(compat: Compat, entries: Vec<(String, T)>) -> Vec<(String, T)> {
    let mut map: BTreeMap<String, T> = BTreeMap::new();
    for (name, entry) in entries {
        map.insert(name, entry);
    }
    let mut entries: Vec<(String, T)> = map.into_iter().collect();
    entries.sort_by(|a, b| compat.compare_names(&a.0, &b.0));
    entries
}

/// `Type.decoder`: a type from its text, qualifiers dropped.
fn parse_type(text: &str) -> Option<Type> {
    let parse = nomi_syntax::parse_type(text);
    if !parse.errors().is_empty() {
        return None;
    }
    let root = parse.syntax();
    let ty = root.children().find_map(ast::Type::cast)?;
    from_ast(&ty)
}

fn from_ast(ty: &ast::Type) -> Option<Type> {
    use ast::Type as T;
    Some(match ty {
        T::ParenType(paren) => return from_ast(&paren.ty()?),
        T::NamedType(named) => {
            let name = named.upper_ident_token()?.text().to_owned();
            let args: Option<Vec<Type>> = named.types().map(|t| from_ast(&t)).collect();
            Type::Type(name, args?)
        }
        T::TypeVar(var) => Type::Var(var.lower_ident_token()?.text().to_owned()),
        T::FunctionType(function) => Type::Lambda(
            Box::new(from_ast(&function.param()?)?),
            Box::new(from_ast(&function.result()?)?),
        ),
        T::UnitType(_) => Type::Unit,
        T::TupleType(tuple) => {
            let mut parts: Vec<Type> =
                tuple.types().map(|t| from_ast(&t)).collect::<Option<_>>()?;
            if parts.len() < 2 {
                return None;
            }
            let rest = parts.split_off(2);
            let b = parts.pop()?;
            let a = parts.pop()?;
            Type::Tuple(Box::new(a), Box::new(b), rest)
        }
        T::RecordType(record) => {
            let fields: Option<Vec<(String, Type)>> = record
                .field_types()
                .map(|f| {
                    Some((
                        f.lower_ident_token()?.text().to_owned(),
                        from_ast(&f.ty()?)?,
                    ))
                })
                .collect();
            let ext = (record.record_extension())
                .and_then(|e| e.lower_ident_token())
                .map(|t| t.text().to_owned());
            Type::Record(fields?, ext)
        }
        T::TupleConstructorType(_) => return None,
    })
}

/// `Changes`: entries added, changed and removed, each kind in its
/// release's order.
struct Changes<'a, T> {
    added: Vec<&'a (String, T)>,
    changed: Vec<Changed<'a, T>>,
    removed: Vec<&'a (String, T)>,
}

/// An entry of both releases that they don't have alike.
struct Changed<'a, T> {
    name: &'a str,
    old: &'a T,
    new: &'a T,
}

/// `getChanges`.
fn changes<'a, T>(
    old: &'a [(String, T)],
    new: &'a [(String, T)],
    equivalent: impl Fn(&T, &T) -> bool,
) -> Changes<'a, T> {
    let by_name = |entries: &'a [(String, T)]| {
        let mut map: HashMap<&str, &T> = HashMap::with_capacity(entries.len());
        for (name, entry) in entries {
            map.entry(name).or_insert(entry);
        }
        map
    };
    let (olds, news) = (by_name(old), by_name(new));
    Changes {
        added: (new.iter())
            .filter(|(name, _)| !olds.contains_key(name.as_str()))
            .collect(),
        changed: (old.iter())
            .filter_map(|(name, old)| {
                let new = news.get(name.as_str())?;
                (!equivalent(old, new)).then_some(Changed { name, old, new })
            })
            .collect(),
        removed: (old.iter())
            .filter(|(name, _)| !news.contains_key(name.as_str()))
            .collect(),
    }
}

/// `ModuleChanges`.
struct ModuleChanges<'a> {
    unions: Changes<'a, Union>,
    aliases: Changes<'a, Alias>,
    values: Changes<'a, Type>,
    binops: Changes<'a, Binop>,
}

/// `PackageChanges`.
pub(crate) struct PackageChanges<'a> {
    added: Vec<&'a str>,
    changed: Vec<(&'a str, ModuleChanges<'a>)>,
    removed: Vec<&'a str>,
}

/// `Magnitude`, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Magnitude {
    Patch,
    Minor,
    Major,
}

impl Magnitude {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Magnitude::Patch => "PATCH",
            Magnitude::Minor => "MINOR",
            Magnitude::Major => "MAJOR",
        }
    }

    /// `bump`: the version after `version` that a change of this
    /// magnitude calls for.
    pub(crate) fn bump(self, version: Version, compat: Compat) -> Version {
        match self {
            Magnitude::Patch => version.bump_patch(compat),
            Magnitude::Minor => version.bump_minor(compat),
            Magnitude::Major => version.bump_major(compat),
        }
    }
}

/// `diff`: the modules added and removed, and those changed beyond a
/// patch.
pub(crate) fn diff<'a>(old: &'a Documentation, new: &'a Documentation) -> PackageChanges<'a> {
    let modules = changes(old, new, |_, _| false);
    let name = |(name, _): &'a (String, Module)| name.as_str();
    PackageChanges {
        added: modules.added.into_iter().map(name).collect(),
        changed: (modules.changed.into_iter())
            .map(|Changed { name, old, new }| (name, diff_module(old, new)))
            .filter(|(_, changes)| module_magnitude(changes) != Magnitude::Patch)
            .collect(),
        removed: modules.removed.into_iter().map(name).collect(),
    }
}

fn diff_module<'a>(old: &'a Module, new: &'a Module) -> ModuleChanges<'a> {
    ModuleChanges {
        unions: changes(&old.unions, &new.unions, equivalent_union),
        aliases: changes(&old.aliases, &new.aliases, |a, b| {
            equivalent(&a.vars, &a.ty, &b.vars, &b.ty)
        }),
        values: changes(&old.values, &new.values, |a, b| equivalent(&[], a, &[], b)),
        binops: changes(&old.binops, &new.binops, |a, b| {
            equivalent(&[], &a.ty, &[], &b.ty)
                && a.associativity == b.associativity
                && a.precedence == b.precedence
        }),
    }
}

/// `isEquivalentUnion`.
fn equivalent_union(old: &Union, new: &Union) -> bool {
    old.ctors.len() == new.ctors.len()
        && (old.ctors.iter().zip(&new.ctors)).all(|((old_name, olds), (new_name, news))| {
            old_name == new_name
                && olds.len() == news.len()
                && (olds.iter().zip(news)).all(|(o, n)| equivalent(&old.vars, o, &new.vars, n))
        })
}

/// `isEquivalentAlias`: whether `new`, over `new_vars`, is `old`, over
/// `old_vars`, with its variables renamed.
fn equivalent(old_vars: &[String], old: &Type, new_vars: &[String], new: &Type) -> bool {
    let mut renamings: Vec<(&str, &str)> = (old_vars.iter().zip(new_vars))
        .map(|(o, n)| (o.as_str(), n.as_str()))
        .collect();
    old_vars.len() == new_vars.len()
        && diff_type(old, new, &mut renamings)
        && equivalent_renaming(&renamings)
}

/// `diffType`: whether the types match, the variable each old one
/// became pushed to `renamings`.
fn diff_type<'a>(old: &'a Type, new: &'a Type, renamings: &mut Vec<(&'a str, &'a str)>) -> bool {
    match (old, new) {
        (Type::Var(o), Type::Var(n)) => {
            renamings.push((o, n));
            true
        }
        (Type::Lambda(a, b), Type::Lambda(x, y)) => {
            diff_type(a, x, renamings) && diff_type(b, y, renamings)
        }
        (Type::Type(o, oargs), Type::Type(n, nargs)) => {
            same_name(o, n) && diff_types(oargs, nargs, renamings)
        }
        (Type::Record(ofields, oext), Type::Record(nfields, next)) => match (oext, next) {
            (None, None) => diff_fields(ofields, nfields, renamings),
            (Some(o), Some(n)) => {
                renamings.push((o, n));
                diff_fields(ofields, nfields, renamings)
            }
            (None, Some(_)) | (Some(_), None) => false,
        },
        (Type::Unit, Type::Unit) => true,
        (Type::Tuple(a, b, cs), Type::Tuple(x, y, zs)) => {
            diff_type(a, x, renamings)
                && diff_type(b, y, renamings)
                && diff_types(cs, zs, renamings)
        }
        _ => false,
    }
}

/// [`diff_type`] of each type of `old` and the one of `new` beside it,
/// which has as many.
fn diff_types<'a>(
    old: &'a [Type],
    new: &'a [Type],
    renamings: &mut Vec<(&'a str, &'a str)>,
) -> bool {
    old.len() == new.len() && (old.iter().zip(new)).all(|(o, n)| diff_type(o, n, renamings))
}

/// `isSameName`: very old docs don't qualify names.
fn same_name(old: &str, new: &str) -> bool {
    if old.contains('.') && new.contains('.') {
        return old == new;
    }
    let [old, new] = [old, new].map(|name| name.rsplit_once('.').map_or(name, |(_, last)| last));
    old == new
}

/// `diffFields`: by field name, which must be the same.
fn diff_fields<'a>(
    old: &'a [(String, Type)],
    new: &'a [(String, Type)],
    renamings: &mut Vec<(&'a str, &'a str)>,
) -> bool {
    if old.len() != new.len() {
        return false;
    }
    let mut old: Vec<&(String, Type)> = old.iter().collect();
    let mut new: Vec<&(String, Type)> = new.iter().collect();
    old.sort_by(|a, b| a.0.cmp(&b.0));
    new.sort_by(|a, b| a.0.cmp(&b.0));
    (old.iter().zip(&new)).all(|(a, b)| a.0 == b.0 && diff_type(&a.1, &b.1, renamings))
}

/// `isEquivalentRenaming`: each old variable became one new one, of a
/// kind that admits what it did, and no two became the same.
fn equivalent_renaming(pairs: &[(&str, &str)]) -> bool {
    let mut renamings: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for &(old, new) in pairs {
        renamings.entry(old).or_default().push(new);
    }
    let mut verified = Vec::new();
    for (old, news) in &renamings {
        let first = news[0];
        if news.iter().any(|n| *n != first) {
            return false;
        }
        verified.push((*old, first));
    }
    let mut targets: Vec<&str> = verified.iter().map(|(_, n)| *n).collect();
    targets.sort_unstable();
    targets.dedup();
    verified.iter().all(|(o, n)| compatible_vars(o, n)) && targets.len() == verified.len()
}

/// `compatibleVars`.
fn compatible_vars(old: &str, new: &str) -> bool {
    use SuperType::{Appendable, CompAppend, Comparable, Number};
    matches!(
        (SuperType::of_name(old), SuperType::of_name(new)),
        (Some(CompAppend), Some(CompAppend))
            | (Some(Comparable | Number), Some(Comparable))
            | (Some(Appendable), Some(Appendable))
            | (Some(Number), Some(Number))
            | (_, None)
    )
}

/// `toMagnitude`.
pub(crate) fn magnitude(changes: &PackageChanges) -> Magnitude {
    let added = if changes.added.is_empty() {
        Magnitude::Patch
    } else {
        Magnitude::Minor
    };
    let removed = if changes.removed.is_empty() {
        Magnitude::Patch
    } else {
        Magnitude::Major
    };
    (changes.changed.iter())
        .map(|(_, c)| module_magnitude(c))
        .fold(added.max(removed), Magnitude::max)
}

/// `moduleChangeMagnitude`.
fn module_magnitude(changes: &ModuleChanges) -> Magnitude {
    [
        change_magnitude(&changes.unions),
        change_magnitude(&changes.aliases),
        change_magnitude(&changes.values),
        change_magnitude(&changes.binops),
    ]
    .into_iter()
    .max()
    .unwrap_or(Magnitude::Patch)
}

fn change_magnitude<T>(changes: &Changes<T>) -> Magnitude {
    if !changes.removed.is_empty() || !changes.changed.is_empty() {
        Magnitude::Major
    } else if !changes.added.is_empty() {
        Magnitude::Minor
    } else {
        Magnitude::Patch
    }
}

/// `toDoc`: the changes as `elm diff` prints them.
fn to_doc(changes: &PackageChanges) -> Doc {
    if changes.added.is_empty() && changes.changed.is_empty() && changes.removed.is_empty() {
        return hsep([
            text("No API changes detected, so this is a"),
            text("PATCH").color(Color::Green),
            text("change."),
        ]);
    }
    let header = hsep([
        text("This is a"),
        text(magnitude(changes).name()).color(Color::Green),
        text("change."),
    ]);
    let mut chunks = Vec::new();
    if !changes.added.is_empty() {
        let names = vcat(changes.added.iter().copied().map(text));
        chunks.push(chunk("ADDED MODULES", Magnitude::Minor, names));
    }
    if !changes.removed.is_empty() {
        let names = vcat(changes.removed.iter().copied().map(text));
        chunks.push(chunk("REMOVED MODULES", Magnitude::Major, names));
    }
    for (name, module) in &changes.changed {
        chunks.push(module_chunk(name, module));
    }
    let mut docs = vec![header, text("")];
    docs.extend(chunks);
    vcat(docs)
}

/// `chunkToDoc`.
fn chunk(title: &str, magnitude: Magnitude, details: Doc) -> Doc {
    let header = text(format!("---- {title} - {} ----", magnitude.name())).color(Color::DullCyan);
    vcat([header, text(""), details.indent(4), text(""), text("")])
}

/// `changesToChunk`.
fn module_chunk(name: &str, changes: &ModuleChanges) -> Doc {
    let unions = sections(&changes.unions, union_doc);
    let aliases = sections(&changes.aliases, alias_doc);
    let values = sections(&changes.values, value_doc);
    let binops = sections(&changes.binops, binop_doc);
    // `changesToDoc`: unions, aliases, operators, then values.
    let category = |title: &str, docs: [Vec<Doc>; 4]| {
        let docs: Vec<Doc> = docs.into_iter().flatten().collect();
        (!docs.is_empty()).then(|| vcat(std::iter::once(text(format!("{title}:"))).chain(docs)))
    };
    let categories = [
        category(
            "Added",
            [unions.added, aliases.added, binops.added, values.added],
        ),
        category(
            "Removed",
            [
                unions.removed,
                aliases.removed,
                binops.removed,
                values.removed,
            ],
        ),
        category(
            "Changed",
            [
                unions.changed,
                aliases.changed,
                binops.changed,
                values.changed,
            ],
        ),
    ];
    let mut spaced = Vec::new();
    for category in categories.into_iter().flatten() {
        if !spaced.is_empty() {
            spaced.push(text(""));
        }
        spaced.push(category);
    }
    chunk(name, module_magnitude(changes), vcat(spaced))
}

/// `changesToDocTriple`: the entries added, changed and removed.
struct Sections {
    added: Vec<Doc>,
    changed: Vec<Doc>,
    removed: Vec<Doc>,
}

fn sections<T>(changes: &Changes<T>, entry: fn(&str, &T) -> Doc) -> Sections {
    let indented = |(name, value): &&(String, T)| entry(name, value).indent(4);
    let diffed = |changed: &Changed<T>| {
        vcat([
            text("  - ").append(entry(changed.name, changed.old)),
            text("  + ").append(entry(changed.name, changed.new)),
            text(""),
        ])
    };
    Sections {
        added: changes.added.iter().map(indented).collect(),
        changed: changes.changed.iter().map(diffed).collect(),
        removed: changes.removed.iter().map(indented).collect(),
    }
}

fn union_doc(name: &str, union: &Union) -> Doc {
    // `<+>` of an empty `hsep` still spaces: `type Body `.
    let setup = text("type")
        .space(text(name))
        .space(hsep(union.vars.iter().map(text)));
    let mut parts = vec![setup];
    for (i, (ctor, types)) in union.ctors.iter().enumerate() {
        let mark = if i == 0 { "=" } else { "|" };
        parts.push(text(mark).space(applied(render::Context::None, ctor, types)));
    }
    sep(parts).hang(4)
}

fn alias_doc(name: &str, alias: &Alias) -> Doc {
    let names = std::iter::once(name).chain(alias.vars.iter().map(String::as_str));
    let declaration = text("type")
        .space(text("alias"))
        .space(hsep(names.map(text)))
        .space(text("="));
    sep([declaration, type_doc(&alias.ty)]).hang(4)
}

fn value_doc(name: &str, ty: &Type) -> Doc {
    sep([text(name).space(text(":")), type_doc(ty)]).hang(4)
}

fn binop_doc(name: &str, binop: &Binop) -> Doc {
    let details = text(format!(
        "    ({}/{})",
        binop.associativity.json_name(),
        binop.precedence
    ))
    .color(Color::Black);
    text(format!("({name})"))
        .space(text(":"))
        .space(type_doc(&binop.ty))
        .append(details)
}

/// `Type.toDoc L.empty Type.None`.
fn type_doc(ty: &Type) -> Doc {
    to_doc_in(render::Context::None, ty)
}

fn to_doc_in(context: render::Context, ty: &Type) -> Doc {
    match ty {
        Type::Lambda(arg, result) => {
            let mut rest = Vec::new();
            let mut at: &Type = result;
            while let Type::Lambda(arg, result) = at {
                rest.push(to_doc_in(render::Context::Func, arg));
                at = result;
            }
            rest.push(to_doc_in(render::Context::Func, at));
            let mut rest = rest.into_iter();
            let second = rest.next().expect("a result at least");
            let first = to_doc_in(render::Context::Func, arg);
            render::lambda(context, first, second, rest.collect())
        }
        Type::Var(x) => text(x),
        Type::Unit => text("()"),
        Type::Tuple(a, b, cs) => render::tuple(
            to_doc_in(render::Context::None, a),
            to_doc_in(render::Context::None, b),
            cs.iter()
                .map(|c| to_doc_in(render::Context::None, c))
                .collect(),
        ),
        Type::Type(name, args) => applied(context, name, args),
        Type::Record(fields, ext) => render::record(
            (fields.iter())
                .map(|(f, t)| (text(f), to_doc_in(render::Context::None, t)))
                .collect(),
            ext.as_ref().map(text),
        ),
    }
}

/// `name` applied to `args`, in `context`.
fn applied(context: render::Context, name: &str, args: &[Type]) -> Doc {
    let args = args.iter().map(|a| to_doc_in(render::Context::App, a));
    render::apply(context, text(name), args.collect())
}

/// What `elm diff` was asked (`Diff.Args`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Args {
    /// The code against the latest release.
    CodeVsLatest,
    /// The code against a release.
    CodeVsExactly(Version),
    /// Two releases of the project's package.
    Local(Version, Version),
    /// Two releases of any package.
    Global(Name, Version, Version),
}

/// `elm diff`'s arguments, as its four forms take them.
pub fn parse_args(args: &[&str]) -> Option<Args> {
    let version = |arg: &str| arg.parse().ok();
    match args {
        [] => Some(Args::CodeVsLatest),
        [v] => version(v).map(Args::CodeVsExactly),
        [v1, v2] => Some(Args::Local(version(v1)?, version(v2)?)),
        [package, v1, v2] => Some(Args::Global(
            package.parse().ok()?,
            version(v1)?,
            version(v2)?,
        )),
        _ => None,
    }
}

/// Why `elm diff`, `elm bump` or `elm publish` stopped: a report, or the
/// build's problems.
pub(crate) enum Stop {
    Report(Report),
    Build(Checked),
}

impl From<Report> for Stop {
    fn from(report: Report) -> Stop {
        Stop::Report(report)
    }
}

impl Stop {
    /// Shows why a command on the project at `root` stopped.
    pub(crate) fn show(&self, console: &mut dyn Console, root: &Path, compat: Compat) {
        match self {
            Stop::Report(report) => console::report(console, report, compat),
            Stop::Build(checked) => check::show_reports(console, root, checked, compat),
        }
    }
}

/// `elm diff`: the changes `args` asks about, printed. Whether nothing
/// was wrong.
pub fn run(cx: &Context<'_, impl Http + Sync>, console: &mut dyn Console, args: &Args) -> bool {
    let root = Root::find(cx.vfs, cx.dir);
    let project = (root.as_ref()).map(|root| Project::read(cx.vfs, root, Personality::Elm));
    let compat = match &project {
        Some(Ok(project)) => project.compat,
        Some(Err(_)) | None => Compat::LATEST,
    };
    match diff_args(cx, project, compat, args) {
        Ok((old, new)) => {
            console::print(console, &to_doc(&diff(&old, &new)), compat);
            console.out("\n");
            true
        }
        Err(stop) => {
            stop.show(console, root.as_ref().map_or(cx.dir, Root::dir), compat);
            false
        }
    }
}

/// The documentation `args` compares, the older first; `project` is the
/// project around the directory, if there is one.
fn diff_args(
    cx: &Context<'_, impl Http + Sync>,
    project: Option<Result<Project, Report>>,
    compat: Compat,
    args: &Args,
) -> Result<(Documentation, Documentation), Stop> {
    let index = latest(
        cx,
        compat,
        "I need the latest list of published packages before I do this diff",
    )?;
    let docs = |name: &Name, version: Version| docs(cx, compat, &index, name, version);
    match args {
        Args::Global(name, v1, v2) => {
            if let Err(nearby) = index.versions_or_nearby(name) {
                return Err(unknown_package(name, &nearby).into());
            }
            Ok((docs(name, *v1.min(v2))?, docs(name, *v1.max(v2))?))
        }
        Args::Local(v1, v2) => {
            let (_, name) = read_outline(project, &index)?;
            Ok((docs(&name, *v1.min(v2))?, docs(&name, *v1.max(v2))?))
        }
        Args::CodeVsLatest => {
            let (project, name) = read_outline(project, &index)?;
            let newest = index.versions(&name).expect("a published package").newest;
            let old = fetch_docs(cx, compat, &name, newest, DIFF)?;
            Ok((old, local_docs(cx, &project)?))
        }
        Args::CodeVsExactly(version) => {
            let (project, name) = read_outline(project, &index)?;
            Ok((docs(&name, *version)?, local_docs(cx, &project)?))
        }
    }
}

/// `Registry.latest`, or the report on why there isn't one.
pub(crate) fn latest(
    cx: &Context<'_, impl Http>,
    compat: Compat,
    context: &str,
) -> Result<Index, Report> {
    Index::latest_strict(cx.http, &cx.cache.index(), compat)
        .map_err(|e| env::latest_problem("PROBLEM UPDATING PACKAGE LIST", context, e))
}

/// `readOutline`: the project, a package the registry knows, and its
/// name.
fn read_outline(
    project: Option<Result<Project, Report>>,
    index: &Index,
) -> Result<(Project, Name), Stop> {
    let Some(project) = project else {
        return Err(no_outline().into());
    };
    let project = project?;
    let name = match &project.outline {
        Outline::App(_) => return Err(application().into()),
        Outline::Pkg(pkg) => pkg.name.clone(),
    };
    if index.versions(&name).is_none() {
        return Err(unpublished().into());
    }
    Ok((project, name))
}

/// `getDocs`: a release's documentation, if the registry lists it.
fn docs(
    cx: &Context<'_, impl Http>,
    compat: Compat,
    index: &Index,
    name: &Name,
    version: Version,
) -> Result<Documentation, Stop> {
    let known = index.versions(name).expect("a known package");
    if !known.all().any(|v| v == version) {
        return Err(unknown_version(version, known.all().collect()).into());
    }
    fetch_docs(cx, compat, name, version, DIFF)
}

/// `Deps.Diff.getDocs`: from the cache, or the website, which is then
/// kept; a cached copy that doesn't read is removed.
pub(crate) fn fetch_docs(
    cx: &Context<'_, impl Http>,
    compat: Compat,
    name: &Name,
    version: Version,
    purpose: &str,
) -> Result<Documentation, Stop> {
    let path = cx.cache.docs(name, version);
    let context = format!("I need the docs for {version} to {purpose}");
    if let Ok(bytes) = std::fs::read(&path) {
        return if let Some(docs) = decode(compat, &bytes) {
            Ok(docs)
        } else {
            let _ = std::fs::remove_file(&path);
            Err(docs_problem(&context, DocsProblem::Cache).into())
        };
    }
    let url = format!(
        "{}/packages/{name}/{version}/docs.json",
        nomi_registry::http::DOMAIN
    );
    let body = (cx.http.get(&url))
        .map_err(|e| docs_problem(&context, DocsProblem::Http(url.clone(), e)))?;
    match decode(compat, &body) {
        Some(docs) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, &body);
            Ok(docs)
        }
        None => Err(docs_problem(&context, DocsProblem::Data(url)).into()),
    }
}

/// What `elm diff` needs docs for.
const DIFF: &str = "compute this diff";

/// `generateDocs`: the project's own documentation, built.
pub(crate) fn local_docs(
    cx: &Context<'_, impl Http + Sync>,
    project: &Project,
) -> Result<Documentation, Stop> {
    Ok(local_docs_json(cx, project)?.0)
}

/// [`local_docs`], and the `docs.json` they read from.
pub(crate) fn local_docs_json(
    cx: &Context<'_, impl Http + Sync>,
    project: &Project,
) -> Result<(Documentation, String), Stop> {
    let compat = project.compat;
    let dir = project.root.dir();
    check_source_dirs(dir, &project.outline)?;
    let (foreigns, interfaces) = foreigns(cx.cache, cx.http, project, Personality::Elm)?;
    if let Outline::Pkg(pkg) = &project.outline
        && pkg.exposed.modules().next().is_none()
    {
        return Err(no_exposed().into());
    }
    let env = env_of(dir, &project.outline, compat, foreigns)?.with_cwd(dir);
    let build = Build {
        snapshot: &cx.vfs.snapshot(),
        env: &env,
        outline: &project.outline,
        paths: &[],
        personality: Personality::Elm,
        packages: (&interfaces).into(),
        artifacts: None,
    };
    let built = (build.run(Goal::Docs)).expect("nothing revises the files while diff builds");
    match built.docs {
        Some(json) => Ok((
            decode(compat, json.as_bytes()).expect("docs nomi wrote"),
            json,
        )),
        None => Err(Stop::Build(built.checked)),
    }
}

/// `DocsProblem`.
enum DocsProblem {
    Cache,
    Http(String, HttpError),
    Data(String),
}

/// `toDocsProblemReport`.
fn docs_problem(context: &str, problem: DocsProblem) -> Report {
    match problem {
        DocsProblem::Cache => Report::new(
            "PROBLEM LOADING DOCS",
            None,
            &format!("{context}, but the local copy seems to be corrupted."),
            [reflow(
                "I deleted the cached version, so the next run should download a fresh copy of \
                 the docs. Hopefully that will get you unstuck, but it will not resolve the root \
                 problem if, for example, a 3rd party editor plugin is modifing cached files for \
                 some reason.",
            )],
        ),
        DocsProblem::Http(url, error) => {
            nomi_registry::report::http_error("PROBLEM LOADING DOCS", &url, &error, context)
        }
        DocsProblem::Data(url) => one_line(
            "PROBLEM LOADING DOCS",
            &format!("{context}, but I got data I cannot read from {url}"),
        ),
    }
}

/// `DiffNoOutline`.
fn no_outline() -> Report {
    Report::new(
        "DIFF WHAT?",
        None,
        "I cannot find an elm.json so I am not sure what you want me to diff. Normally you \
         run `elm diff` from within a project!",
        [
            reflow("If you are just curious to see a diff, try running this command:"),
            text("elm diff elm/http 1.0.0 2.0.0")
                .color(Color::Green)
                .indent(4),
        ],
    )
}

/// `DiffApplication`.
fn application() -> Report {
    Report::new(
        "CANNOT DIFF APPLICATIONS",
        elm_json_path(),
        "Your elm.json says this project is an application, but `elm diff` only works with \
         packages. That way there are previously published versions of the API to diff \
         against!",
        [
            reflow("If you are just curious to see a diff, try running this command:"),
            text("elm diff elm/json 1.0.0 1.1.2")
                .color(Color::DullYellow)
                .indent(4),
        ],
    )
}

/// `DiffNoExposed`.
fn no_exposed() -> Report {
    Report::new(
        "NO EXPOSED MODULES",
        elm_json_path(),
        "Your elm.json has no \"exposed-modules\" which means there is no public API at all \
         right now! What am I supposed to diff?",
        [reflow(
            "Try adding some modules back to the \"exposed-modules\" field.",
        )],
    )
}

/// `DiffUnpublished`.
fn unpublished() -> Report {
    one_line(
        "UNPUBLISHED",
        "This package is not published yet. There is nothing to diff against!",
    )
}

/// `DiffUnknownPackage`.
fn unknown_package(name: &Name, nearby: &[&Name]) -> Report {
    Report::new(
        "UNKNOWN PACKAGE",
        None,
        "I cannot find a package called:",
        [
            text(name.to_string()).color(Color::Red).indent(4),
            text("Maybe you want one of these instead?"),
            vcat(nearby.iter().map(|n| text(n.to_string())))
                .color(Color::DullYellow)
                .indent(4),
            text("But check <https://package.elm-lang.org> to see all possibilities!"),
        ],
    )
}

/// `DiffUnknownVersion`: the releases grouped by major version.
fn unknown_version(version: Version, mut published: Vec<Version>) -> Report {
    published.sort();
    let mut rows: Vec<Vec<Version>> = Vec::new();
    for v in published {
        match rows.last_mut() {
            Some(row) if row[0].major == v.major => row.push(v),
            _ => rows.push(vec![v]),
        }
    }
    let message = [text("Version"), text(version.to_string()).color(Color::Red)]
        .into_iter()
        .chain(words(
            "has never been published, so I cannot diff against it.",
        ));
    Report::doc(
        "UNKNOWN VERSION",
        None,
        fill_sep(message),
        [
            text("Here are all the versions that HAVE been published:"),
            vcat(
                rows.iter()
                    .map(|row| hsep(row.iter().map(|v| text(v.to_string())))),
            )
            .color(Color::DullYellow)
            .indent(4),
            text("Want one of those instead?"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(ty: &str) -> Type {
        parse_type(ty).unwrap_or_else(|| panic!("{ty} doesn't parse"))
    }

    fn same(old: &str, new: &str) -> bool {
        equivalent(&[], &value(old), &[], &value(new))
    }

    #[test]
    fn renamed_variables_are_the_same_type() {
        assert!(same("a -> b -> a", "x -> y -> x"));
        assert!(!same("a -> b -> a", "x -> x -> x"));
        assert!(!same("a -> a", "x -> y"));
    }

    #[test]
    fn constrained_variables_keep_their_kind() {
        assert!(same("number -> number", "number1 -> number1"));
        assert!(same("number -> number", "comparable -> comparable"));
        assert!(!same("comparable -> comparable", "number -> number"));
        assert!(same("comparable -> comparable", "a -> a"));
        assert!(!same("a -> a", "comparable -> comparable"));
    }

    #[test]
    fn qualifiers_and_field_order_do_not_matter() {
        assert_eq!(value("Json.Decode.Decoder a"), value("Decoder a"));
        assert!(same("{ a : Int, b : Float }", "{ b : Float, a : Int }"));
        assert!(!same("{ a : Int }", "{ r | a : Int }"));
    }

    #[test]
    fn magnitudes_follow_the_largest_change() {
        let docs = |values: &[(&str, &str)]| -> Documentation {
            let module = Module {
                values: values
                    .iter()
                    .map(|(n, t)| (n.to_string(), value(t)))
                    .collect(),
                ..Module::default()
            };
            vec![("M".to_owned(), module)]
        };
        let old = docs(&[("f", "Int -> Int")]);
        let added = docs(&[("f", "Int -> Int"), ("g", "Int")]);
        let changed = docs(&[("f", "Float -> Int")]);
        assert_eq!(magnitude(&diff(&old, &old)), Magnitude::Patch);
        assert_eq!(magnitude(&diff(&old, &added)), Magnitude::Minor);
        assert_eq!(magnitude(&diff(&old, &changed)), Magnitude::Major);
        assert_eq!(magnitude(&diff(&added, &old)), Magnitude::Major);
    }
}
