//! `elm diff` (`Diff.hs`) and the API diff it and `elm bump` compute
//! (`Deps/Diff.hs`): two releases' documentation compared module by
//! module, each value, union, alias and operator added, removed or
//! changed, a change ignoring how type variables are named, and the
//! magnitude of the whole. Each list is in the release's order of names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_base::diagnostic::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, hsep, reflow, sep, text, vcat};
use nomi_compiler::optimized::binary::names;
use nomi_compiler::types::render::{self, Context};
use nomi_project::version::Version;
use nomi_project::{Name, Outline};
use nomi_registry::{Http, HttpError, Index, LatestError};
use nomi_syntax::ast::{self, AstNode};
use serde_json::Value as Json;

use crate::Vfs;
use crate::check::{self, Checked, Style, env_of, foreigns};
use crate::console::{self, Console, Personality};
use crate::paths::{Cache, Root};
use crate::project::{Project, check_source_dirs};

/// `Elm.Compiler.Type.Type`: a type as documentation writes it, its names
/// unqualified as the decoder reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Lambda(Box<Type>, Box<Type>),
    Var(String),
    Type(String, Vec<Type>),
    Record(Vec<(String, Type)>, Option<String>),
    Unit,
    Tuple(Box<Type>, Box<Type>, Vec<Type>),
}

/// `Docs.Union`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Union {
    pub vars: Vec<String>,
    pub ctors: Vec<(String, Vec<Type>)>,
}

/// `Docs.Alias`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub vars: Vec<String>,
    pub ty: Type,
}

/// `Docs.Binop`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binop {
    pub ty: Type,
    pub associativity: String,
    pub precedence: i64,
}

/// `Docs.Module`, each kind of entry by name in the release's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub unions: Vec<(String, Union)>,
    pub aliases: Vec<(String, Alias)>,
    pub values: Vec<(String, Type)>,
    pub binops: Vec<(String, Binop)>,
}

/// `Docs.Documentation`: the modules by name in the release's order.
pub type Documentation = Vec<(String, Module)>;

/// `Docs.decoder`: `docs.json` read, its types parsed; `None` for
/// anything else.
pub fn decode(compat: Compat, bytes: &[u8]) -> Option<Documentation> {
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
                associativity: binop["associativity"].as_str()?.to_owned(),
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
    entries.sort_by(|a, b| names(compat, &a.0, &b.0));
    entries
}

/// `Type.decoder`: a type from its text, qualifiers dropped.
pub fn parse_type(text: &str) -> Option<Type> {
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

/// `Changes`: entries added, changed (old and new) and removed.
#[derive(Clone, Debug)]
pub struct Changes<T> {
    pub added: Vec<(String, T)>,
    pub changed: Vec<(String, (T, T))>,
    pub removed: Vec<(String, T)>,
}

/// `getChanges`.
fn changes<T: Clone>(
    old: &[(String, T)],
    new: &[(String, T)],
    equivalent: impl Fn(&T, &T) -> bool,
) -> Changes<T> {
    let find = |list: &[(String, T)], name: &str| {
        list.iter().find(|(n, _)| n == name).map(|(_, e)| e.clone())
    };
    Changes {
        added: (new.iter())
            .filter(|(n, _)| find(old, n).is_none())
            .cloned()
            .collect(),
        changed: (old.iter())
            .filter_map(|(n, o)| {
                let new = find(new, n)?;
                (!equivalent(o, &new)).then(|| (n.clone(), (o.clone(), new)))
            })
            .collect(),
        removed: (old.iter())
            .filter(|(n, _)| find(new, n).is_none())
            .cloned()
            .collect(),
    }
}

/// `ModuleChanges`.
#[derive(Clone, Debug)]
pub struct ModuleChanges {
    pub unions: Changes<Union>,
    pub aliases: Changes<Alias>,
    pub values: Changes<Type>,
    pub binops: Changes<Binop>,
}

/// `PackageChanges`.
#[derive(Clone, Debug)]
pub struct PackageChanges {
    pub added: Vec<String>,
    pub changed: Vec<(String, ModuleChanges)>,
    pub removed: Vec<String>,
}

/// `Magnitude`, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Magnitude {
    Patch,
    Minor,
    Major,
}

impl Magnitude {
    pub fn name(self) -> &'static str {
        match self {
            Magnitude::Patch => "PATCH",
            Magnitude::Minor => "MINOR",
            Magnitude::Major => "MAJOR",
        }
    }
}

/// `diff`: the modules added and removed, and those changed beyond a
/// patch.
pub fn diff(old: &Documentation, new: &Documentation) -> PackageChanges {
    let modules = changes(old, new, |_, _| false);
    PackageChanges {
        added: modules.added.into_iter().map(|(n, _)| n).collect(),
        changed: (modules.changed.into_iter())
            .map(|(name, (old, new))| (name, diff_module(&old, &new)))
            .filter(|(_, changes)| module_magnitude(changes) != Magnitude::Patch)
            .collect(),
        removed: modules.removed.into_iter().map(|(n, _)| n).collect(),
    }
}

fn diff_module(old: &Module, new: &Module) -> ModuleChanges {
    ModuleChanges {
        unions: changes(&old.unions, &new.unions, equivalent_union),
        aliases: changes(&old.aliases, &new.aliases, equivalent_alias),
        values: changes(&old.values, &new.values, |a, b| {
            equivalent_alias(&alias(a), &alias(b))
        }),
        binops: changes(&old.binops, &new.binops, |a, b| {
            equivalent_alias(&alias(&a.ty), &alias(&b.ty))
                && a.associativity == b.associativity
                && a.precedence == b.precedence
        }),
    }
}

fn alias(ty: &Type) -> Alias {
    Alias {
        vars: Vec::new(),
        ty: ty.clone(),
    }
}

/// `isEquivalentUnion`.
fn equivalent_union(old: &Union, new: &Union) -> bool {
    old.ctors.len() == new.ctors.len()
        && old.ctors.iter().zip(&new.ctors).all(|(a, b)| a.0 == b.0)
        && old
            .ctors
            .iter()
            .zip(&new.ctors)
            .all(|((_, old_types), (_, new_types))| {
                old_types.len() == new_types.len()
                    && old_types.iter().zip(new_types).all(|(o, n)| {
                        let o = Alias {
                            vars: old.vars.clone(),
                            ty: o.clone(),
                        };
                        let n = Alias {
                            vars: new.vars.clone(),
                            ty: n.clone(),
                        };
                        equivalent_alias(&o, &n)
                    })
            })
}

/// `isEquivalentAlias`.
fn equivalent_alias(old: &Alias, new: &Alias) -> bool {
    match diff_type(&old.ty, &new.ty) {
        None => false,
        Some(renamings) => {
            let mut pairs: Vec<(String, String)> = old
                .vars
                .iter()
                .cloned()
                .zip(new.vars.iter().cloned())
                .collect();
            pairs.extend(renamings);
            old.vars.len() == new.vars.len() && equivalent_renaming(&pairs)
        }
    }
}

/// `diffType`: the variables each old one became, if the types match.
fn diff_type(old: &Type, new: &Type) -> Option<Vec<(String, String)>> {
    match (old, new) {
        (Type::Var(o), Type::Var(n)) => Some(vec![(o.clone(), n.clone())]),
        (Type::Lambda(a, b), Type::Lambda(x, y)) => {
            let mut vars = diff_type(a, x)?;
            vars.extend(diff_type(b, y)?);
            Some(vars)
        }
        (Type::Type(o, oargs), Type::Type(n, nargs)) => {
            if !same_name(o, n) || oargs.len() != nargs.len() {
                return None;
            }
            let mut vars = Vec::new();
            for (a, b) in oargs.iter().zip(nargs) {
                vars.extend(diff_type(a, b)?);
            }
            Some(vars)
        }
        (Type::Record(ofields, oext), Type::Record(nfields, next)) => match (oext, next) {
            (None, Some(_)) | (Some(_), None) => None,
            (None, None) => diff_fields(ofields, nfields),
            (Some(o), Some(n)) => {
                let mut vars = vec![(o.clone(), n.clone())];
                vars.extend(diff_fields(ofields, nfields)?);
                Some(vars)
            }
        },
        (Type::Unit, Type::Unit) => Some(Vec::new()),
        (Type::Tuple(a, b, cs), Type::Tuple(x, y, zs)) => {
            if cs.len() != zs.len() {
                return None;
            }
            let mut vars = diff_type(a, x)?;
            vars.extend(diff_type(b, y)?);
            for (c, z) in cs.iter().zip(zs) {
                vars.extend(diff_type(c, z)?);
            }
            Some(vars)
        }
        _ => None,
    }
}

/// `isSameName`: very old docs don't qualify names.
fn same_name(old: &str, new: &str) -> bool {
    let old_parts: Vec<&str> = old.rsplit('.').collect();
    let new_parts: Vec<&str> = new.rsplit('.').collect();
    match (old_parts.as_slice(), new_parts.as_slice()) {
        ([o], [n, ..]) | ([o, ..], [n]) => o == n,
        _ => old == new,
    }
}

/// `diffFields`: by field name, which must be the same.
fn diff_fields(old: &[(String, Type)], new: &[(String, Type)]) -> Option<Vec<(String, String)>> {
    if old.len() != new.len() {
        return None;
    }
    let mut old: Vec<&(String, Type)> = old.iter().collect();
    let mut new: Vec<&(String, Type)> = new.iter().collect();
    old.sort_by(|a, b| a.0.cmp(&b.0));
    new.sort_by(|a, b| a.0.cmp(&b.0));
    if old.iter().zip(&new).any(|(a, b)| a.0 != b.0) {
        return None;
    }
    let mut vars = Vec::new();
    for (a, b) in old.iter().zip(&new) {
        vars.extend(diff_type(&a.1, &b.1)?);
    }
    Some(vars)
}

/// `isEquivalentRenaming`: each old variable became one new one, of a
/// kind that admits what it did, and no two became the same.
fn equivalent_renaming(pairs: &[(String, String)]) -> bool {
    let mut renamings: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (old, new) in pairs {
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
    use Category::*;
    matches!(
        (category(old), category(new)),
        (CompAppend, CompAppend)
            | (Comparable, Comparable)
            | (Appendable, Appendable)
            | (Number, Number)
            | (Number, Comparable)
            | (_, Any)
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    CompAppend,
    Comparable,
    Appendable,
    Number,
    Any,
}

fn category(name: &str) -> Category {
    if name.starts_with("compappend") {
        Category::CompAppend
    } else if name.starts_with("comparable") {
        Category::Comparable
    } else if name.starts_with("appendable") {
        Category::Appendable
    } else if name.starts_with("number") {
        Category::Number
    } else {
        Category::Any
    }
}

/// `toMagnitude`.
pub fn magnitude(changes: &PackageChanges) -> Magnitude {
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
pub fn module_magnitude(changes: &ModuleChanges) -> Magnitude {
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

/// `bump`: the version the changes call for after `version`.
pub fn bump(compat: Compat, changes: &PackageChanges, version: Version) -> Version {
    match magnitude(changes) {
        Magnitude::Patch => version.bump_patch(compat),
        Magnitude::Minor => version.bump_minor(compat),
        Magnitude::Major => version.bump_major(compat),
    }
}

/// `toDoc`: the changes as `elm diff` prints them.
pub fn to_doc(changes: &PackageChanges) -> Doc {
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
        let names = vcat(changes.added.iter().map(text));
        chunks.push(chunk("ADDED MODULES", Magnitude::Minor, names));
    }
    if !changes.removed.is_empty() {
        let names = vcat(changes.removed.iter().map(text));
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
    let (union_add, union_change, union_remove) = triple(&changes.unions, union_doc);
    let (alias_add, alias_change, alias_remove) = triple(&changes.aliases, alias_doc);
    let (value_add, value_change, value_remove) = triple(&changes.values, value_doc);
    let (binop_add, binop_change, binop_remove) = triple(&changes.binops, binop_doc);
    let sections: Vec<Doc> = [
        category_doc("Added", union_add, alias_add, value_add, binop_add),
        category_doc(
            "Removed",
            union_remove,
            alias_remove,
            value_remove,
            binop_remove,
        ),
        category_doc(
            "Changed",
            union_change,
            alias_change,
            value_change,
            binop_change,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut spaced = Vec::new();
    for (i, section) in sections.into_iter().enumerate() {
        if i > 0 {
            spaced.push(text(""));
        }
        spaced.push(section);
    }
    chunk(name, module_magnitude(changes), vcat(spaced))
}

/// `changesToDocTriple`.
fn triple<T>(changes: &Changes<T>, entry: fn(&str, &T) -> Doc) -> (Vec<Doc>, Vec<Doc>, Vec<Doc>) {
    let indented = |(name, value): &(String, T)| entry(name, value).indent(4);
    let diffed = |(name, (old, new)): &(String, (T, T))| {
        vcat([
            text("  - ").append(entry(name, old)),
            text("  + ").append(entry(name, new)),
            text(""),
        ])
    };
    (
        changes.added.iter().map(indented).collect(),
        changes.changed.iter().map(diffed).collect(),
        changes.removed.iter().map(indented).collect(),
    )
}

/// `changesToDoc`: unions, aliases, operators, then values.
fn category_doc(
    category: &str,
    unions: Vec<Doc>,
    aliases: Vec<Doc>,
    values: Vec<Doc>,
    binops: Vec<Doc>,
) -> Option<Doc> {
    if unions.is_empty() && aliases.is_empty() && values.is_empty() && binops.is_empty() {
        return None;
    }
    let mut docs = vec![text(format!("{category}:"))];
    docs.extend(unions);
    docs.extend(aliases);
    docs.extend(binops);
    docs.extend(values);
    Some(vcat(docs))
}

fn union_doc(name: &str, union: &Union) -> Doc {
    // `<+>` of an empty `hsep` still spaces: `type Body `.
    let setup = text("type")
        .space(text(name))
        .space(hsep(union.vars.iter().map(text)));
    let mut parts = vec![setup];
    for (i, (ctor, types)) in union.ctors.iter().enumerate() {
        let mark = if i == 0 { "=" } else { "|" };
        let ctor = type_doc(&Type::Type(ctor.clone(), types.clone()));
        parts.push(text(mark).space(ctor));
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
        binop.associativity, binop.precedence
    ))
    .color(Color::Black);
    text(format!("({name})"))
        .space(text(":"))
        .space(type_doc(&binop.ty))
        .append(details)
}

/// `Type.toDoc L.empty Type.None`.
pub fn type_doc(ty: &Type) -> Doc {
    to_doc_in(Context::None, ty)
}

fn to_doc_in(context: Context, ty: &Type) -> Doc {
    match ty {
        Type::Lambda(..) => {
            let mut parts = Vec::new();
            let mut at = ty;
            while let Type::Lambda(arg, result) = at {
                parts.push(to_doc_in(Context::Func, arg));
                at = result;
            }
            parts.push(to_doc_in(Context::Func, at));
            let mut parts = parts.into_iter();
            let a = parts.next().unwrap();
            let b = parts.next().unwrap();
            render::lambda(context, a, b, parts.collect())
        }
        Type::Var(x) => text(x),
        Type::Unit => text("()"),
        Type::Tuple(a, b, cs) => render::tuple(
            to_doc_in(Context::None, a),
            to_doc_in(Context::None, b),
            cs.iter().map(|c| to_doc_in(Context::None, c)).collect(),
        ),
        Type::Type(name, args) => render::apply(
            context,
            text(name),
            args.iter().map(|a| to_doc_in(Context::App, a)).collect(),
        ),
        Type::Record(fields, ext) => render::record(
            (fields.iter())
                .map(|(f, t)| (text(f), to_doc_in(Context::None, t)))
                .collect(),
            ext.as_ref().map(text),
        ),
    }
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
    let compat = Compat::LATEST;
    let cursor = || nomi_project::json::Cursor::start(compat);
    let version = |arg: &str| Version::parse(arg.as_bytes(), cursor(), compat).ok();
    let name = |arg: &str| Name::parse(arg.as_bytes(), cursor(), compat).ok();
    match args {
        [] => Some(Args::CodeVsLatest),
        [v] => version(v).map(Args::CodeVsExactly),
        [v1, v2] => Some(Args::Local(version(v1)?, version(v2)?)),
        [package, v1, v2] => Some(Args::Global(name(package)?, version(v1)?, version(v2)?)),
        _ => None,
    }
}

/// Why `elm diff` stopped: a report, or the build's problems.
pub(crate) enum Stop {
    Report(Report),
    Build(Checked),
}

impl From<Report> for Stop {
    fn from(report: Report) -> Stop {
        Stop::Report(report)
    }
}

/// `elm diff`: the changes `args` asks about, printed. Whether nothing
/// was wrong.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &(impl Http + Sync),
    args: &Args,
) -> bool {
    let root = Root::find(vfs, dir);
    let compat = (root.as_ref())
        .and_then(|root| Project::read(vfs, root, Personality::Elm).ok())
        .map_or(Compat::LATEST, |p| p.compat);
    match diff_args(vfs, root.as_ref(), cache, http, compat, args) {
        Ok(changes) => {
            console::print(console, &to_doc(&changes), compat);
            console.out("\n");
            true
        }
        Err(Stop::Report(report)) => {
            console::report(console, &report, compat);
            false
        }
        Err(Stop::Build(checked)) => {
            let root = root.map_or(dir.to_owned(), |r| r.dir().to_owned());
            check::show(console, &root, &checked, compat, Style::Terminal);
            false
        }
    }
}

fn diff_args(
    vfs: &Vfs,
    root: Option<&Root>,
    cache: &Cache,
    http: &(impl Http + Sync),
    compat: Compat,
    args: &Args,
) -> Result<PackageChanges, Stop> {
    let index = latest(
        http,
        cache,
        compat,
        "I need the latest list of published packages before I do this diff",
    )?;
    let docs = |name: &Name, version: Version| docs(cache, http, compat, &index, name, version);
    match args {
        Args::Global(name, v1, v2) => {
            if let Err(nearby) = index.versions_or_nearby(name) {
                let nearby = nearby.into_iter().cloned().collect();
                return Err(unknown_package(name, nearby).into());
            }
            let old = docs(name, *v1.min(v2))?;
            let new = docs(name, *v1.max(v2))?;
            Ok(diff(&old, &new))
        }
        Args::Local(v1, v2) => {
            let (_, name) = read_outline(vfs, root, &index)?;
            let old = docs(&name, *v1.min(v2))?;
            let new = docs(&name, *v1.max(v2))?;
            Ok(diff(&old, &new))
        }
        Args::CodeVsLatest | Args::CodeVsExactly(_) => {
            let (project, name) = read_outline(vfs, root, &index)?;
            let version = match args {
                Args::CodeVsExactly(version) => *version,
                _ => index.versions(&name).expect("a published package").newest,
            };
            let old = match args {
                Args::CodeVsExactly(_) => docs(&name, version)?,
                _ => fetch_docs(cache, http, compat, &name, version, DIFF)?,
            };
            let root = root.expect("a project");
            let new = local_docs(vfs, root, &project, cache, http)?;
            Ok(diff(&old, &new))
        }
    }
}

/// `Registry.latest`, or the report on why there isn't one.
pub(crate) fn latest(
    http: &impl Http,
    cache: &Cache,
    compat: Compat,
    context: &str,
) -> Result<Index, Report> {
    const TITLE: &str = "PROBLEM UPDATING PACKAGE LIST";
    match Index::latest_strict(http, &cache.index(), compat) {
        Ok(index) => Ok(index),
        Err(LatestError::Registry(problem)) => Err(nomi_registry::report::registry_problem(
            TITLE, &problem, context,
        )),
        Err(LatestError::Io(e)) => Err(Report::new(
            TITLE,
            None,
            &format!("{context}, but I could not read or write the copy kept in the cache: {e}"),
            [],
        )),
    }
}

/// `readOutline`: the project, a package the registry knows.
fn read_outline(vfs: &Vfs, root: Option<&Root>, index: &Index) -> Result<(Project, Name), Stop> {
    let Some(root) = root else {
        return Err(no_outline().into());
    };
    let project = Project::read(vfs, root, Personality::Elm)?;
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
    cache: &Cache,
    http: &impl Http,
    compat: Compat,
    index: &Index,
    name: &Name,
    version: Version,
) -> Result<Documentation, Stop> {
    let known = index.versions(name).expect("a known package");
    if !known.all().any(|v| v == version) {
        return Err(unknown_version(version, known.all().collect()).into());
    }
    fetch_docs(cache, http, compat, name, version, DIFF)
}

/// `Deps.Diff.getDocs`: from the cache, or the website, which is then
/// kept; a cached copy that doesn't read is removed.
pub(crate) fn fetch_docs(
    cache: &Cache,
    http: &impl Http,
    compat: Compat,
    name: &Name,
    version: Version,
    purpose: &str,
) -> Result<Documentation, Stop> {
    let path = docs_path(cache, name, version);
    let context = format!("I need the docs for {version} to {purpose}");
    if let Ok(bytes) = std::fs::read(&path) {
        return match decode(compat, &bytes) {
            Some(docs) => Ok(docs),
            None => {
                let _ = std::fs::remove_file(&path);
                Err(docs_problem(&context, DocsProblem::Cache).into())
            }
        };
    }
    let url = format!(
        "{}/packages/{name}/{version}/docs.json",
        nomi_registry::http::DOMAIN
    );
    let body =
        (http.get(&url)).map_err(|e| docs_problem(&context, DocsProblem::Http(url.clone(), e)))?;
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

/// Where a release's documentation is kept.
fn docs_path(cache: &Cache, name: &Name, version: Version) -> PathBuf {
    cache
        .dir()
        .join("docs")
        .join(name.to_string())
        .join(version.to_string())
        .join("docs.json")
}

/// `generateDocs`: the project's own documentation, built.
pub(crate) fn local_docs(
    vfs: &Vfs,
    root: &Root,
    project: &Project,
    cache: &Cache,
    http: &(impl Http + Sync),
) -> Result<Documentation, Stop> {
    let compat = project.compat;
    let dir = root.dir();
    check_source_dirs(dir, &project.outline)?;
    let (foreigns, interfaces, _) = foreigns(cache, http, project, Personality::Elm)?;
    if let Outline::Pkg(pkg) = &project.outline
        && pkg.exposed.modules().next().is_none()
    {
        return Err(no_exposed().into());
    }
    let env = env_of(dir, &project.outline, compat, foreigns)?.with_cwd(dir);
    let (checked, json, _) = check::docs(
        &vfs.snapshot(),
        &env,
        &project.outline,
        Personality::Elm,
        &interfaces,
    );
    match json {
        Some(json) => Ok(decode(compat, json.as_bytes()).expect("docs nomi wrote")),
        None => Err(Stop::Build(checked)),
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
        DocsProblem::Data(url) => Report::new(
            "PROBLEM LOADING DOCS",
            None,
            &format!("{context}, but I got data I cannot read from {url}"),
            [],
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
        Some("elm.json".to_owned()),
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
        Some("elm.json".to_owned()),
        "Your elm.json has no \"exposed-modules\" which means there is no public API at all \
         right now! What am I supposed to diff?",
        [reflow(
            "Try adding some modules back to the \"exposed-modules\" field.",
        )],
    )
}

/// `DiffUnpublished`.
fn unpublished() -> Report {
    Report::new(
        "UNPUBLISHED",
        None,
        "This package is not published yet. There is nothing to diff against!",
        [],
    )
}

/// `DiffUnknownPackage`.
fn unknown_package(name: &Name, nearby: Vec<Name>) -> Report {
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
    let mut words: Vec<Doc> = vec![text("Version"), text(version.to_string()).color(Color::Red)];
    words.extend(
        "has never been published, so I cannot diff against it."
            .split_whitespace()
            .map(text),
    );
    Report::doc(
        "UNKNOWN VERSION",
        None,
        fill_sep(words),
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
        equivalent_alias(&alias(&value(old)), &alias(&value(new)))
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
