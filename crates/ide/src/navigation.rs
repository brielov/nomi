//! Go to definition, find references and document highlights (plan
//! 5.16), from the references canonicalization records for each module
//! of the project, in the index of the project that owns the file
//! (`nomi_engine::index`). A name's range is its own
//! token's: `x` in `A.x`, `+` in `(+)`. A local variable is found from
//! its uses, so one never used has nothing to show.

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_compiler::canonical::Home;
use nomi_compiler::canonicalize::references::{Kind, Target};
use nomi_compiler::hir::{self, Def};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, parse_module};

/// A range in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub range: TextRange,
}

/// Where the name at `offset` in the file at `path` is defined.
pub fn definition(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Option<Location> {
    let module = index.module(path)?;
    let target = target_at(index, module, offset)?;
    declaration(snapshot, index, module, &target)
}

/// Every use of the name at `offset` in the file at `path`, across the
/// project, in the order of the project's modules and then of the text;
/// its definition first where `with_declaration`.
pub fn references(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
    with_declaration: bool,
) -> Vec<Location> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    let Some(target) = target_at(index, module, offset) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    if with_declaration {
        found.extend(declaration(snapshot, index, module, &target));
    }
    // A local is one module's.
    let modules: Vec<&Indexed> = match target {
        Target::Local(_) => vec![module],
        Target::Global { .. } => index.modules.iter().collect(),
    };
    for m in modules {
        found.extend(uses(m, &target).map(|range| Location {
            path: m.path.clone(),
            range,
        }));
    }
    found
}

/// The name at `offset` in the file at `path`, everywhere in that file,
/// its definition included where it is there.
pub fn highlights(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Vec<TextRange> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    let Some(target) = target_at(index, module, offset) else {
        return Vec::new();
    };
    let mut found: Vec<TextRange> = (declaration(snapshot, index, module, &target))
        .filter(|d| d.path == module.path)
        .map(|d| d.range)
        .into_iter()
        .collect();
    found.extend(uses(module, &target));
    found.sort_by_key(|r| r.start());
    found.dedup();
    found
}

/// The ranges in `module` of the names that refer to `target`.
fn uses<'m>(module: &'m Indexed, target: &'m Target) -> impl Iterator<Item = TextRange> + 'm {
    (module.references.iter())
        .filter(move |r| r.target == *target)
        .filter_map(|r| name_range(&module.root, r.range, kind(&r.target)))
}

fn kind(target: &Target) -> Option<Kind> {
    match target {
        Target::Local(_) => None,
        Target::Global { kind, .. } => Some(*kind),
    }
}

/// What the name at `offset` refers to, or names where it is defined:
/// a reference, a top-level declaration, or a local's binding.
fn target_at(index: &Index, module: &Indexed, offset: TextSize) -> Option<Target> {
    let at = |range: Option<TextRange>| range.is_some_and(|r| r.contains_inclusive(offset));
    let used =
        (module.references.iter()).find(|r| at(name_range(&module.root, r.range, kind(&r.target))));
    if let Some(reference) = used {
        return Some(reference.target.clone());
    }
    let home = Home {
        package: index.package.clone().into(),
        module: module.name.clone().into(),
    };
    let declared = declarations(&module.hir).find(|(_, _, range)| range.contains_inclusive(offset));
    if let Some((name, kind, _)) = declared {
        return Some(Target::Global { home, name, kind });
    }
    (module.references.iter()).find_map(|r| match r.target {
        Target::Local(binder) if at(name_range(&module.root, binder, None)) => {
            Some(r.target.clone())
        }
        _ => None,
    })
}

/// Where `target`, which a name in `module` refers to, is defined.
fn declaration(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    target: &Target,
) -> Option<Location> {
    let (home, name, kind) = match target {
        Target::Local(binder) => {
            return Some(Location {
                path: module.path.clone(),
                range: name_range(&module.root, *binder, None)?,
            });
        }
        Target::Global { home, name, kind } => (home, name, *kind),
    };
    if home.package == index.package {
        let declaring = index.named(&home.module)?;
        return Some(Location {
            path: declaring.path.clone(),
            range: declared(&declaring.hir, name, kind)?,
        });
    }
    // A dependency's module, read from its sources.
    let mut path = index.packages.get(&*home.package)?.join("src");
    path.extend(home.module.split('.'));
    path.set_extension("elm");
    let text = snapshot.read(&path).ok()?;
    let parse = parse_module(&text);
    let hir = hir::lower(&ast::Module::cast(parse.syntax())?);
    Some(Location {
        range: declared(&hir, name, kind).unwrap_or_default(),
        path,
    })
}

/// The range of the name `name`, a `kind`, that `module` declares at the
/// top level; for a module, its name in the header.
fn declared(module: &hir::Module, name: &str, kind: Kind) -> Option<TextRange> {
    if kind == Kind::Module {
        return Some(module.name.as_ref().map(|n| n.range).unwrap_or_default());
    }
    // A record alias is its own constructor.
    let found = declarations(module).find(|(n, k, _)| n == name && *k == kind);
    let alias = || {
        (kind == Kind::Ctor)
            .then(|| module.aliases.iter().find(|a| a.name.text == name))
            .flatten()
            .map(|a| a.name.range)
    };
    found.map(|(_, _, range)| range).or_else(alias)
}

/// Each top-level name `module` declares, its kind and its range.
fn declarations(module: &hir::Module) -> impl Iterator<Item = (String, Kind, TextRange)> + '_ {
    let spanned = |s: &hir::Spanned, kind| (s.text.clone(), kind, s.range);
    let values = module
        .values
        .iter()
        .filter_map(move |&id| match &module.defs[id] {
            Def::Define { name, .. } => Some(spanned(name, Kind::Value)),
            Def::Destruct { .. } => None,
        });
    let unions = module.unions.iter().flat_map(move |u| {
        std::iter::once(spanned(&u.name, Kind::Type))
            .chain(u.variants.iter().map(move |(v, _)| spanned(v, Kind::Ctor)))
    });
    values
        .chain(unions)
        .chain(
            module
                .aliases
                .iter()
                .map(move |a| spanned(&a.name, Kind::Type)),
        )
        .chain(
            module
                .ports
                .iter()
                .map(move |p| spanned(&p.name, Kind::Value)),
        )
        .chain(
            module
                .binops
                .iter()
                .map(move |b| spanned(&b.op, Kind::Binop)),
        )
}

/// The name in `range`, which may hold trivia before it and a qualifier
/// or parentheses around it: its last token. A module's name is all of
/// its parts.
fn name_range(root: &SyntaxNode, range: TextRange, kind: Option<Kind>) -> Option<TextRange> {
    let range = range.intersect(root.text_range())?;
    let node = match root.covering_element(range) {
        SyntaxElement::Node(node) => node,
        SyntaxElement::Token(token) => token.parent()?,
    };
    let tokens: Vec<_> = (node.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| range.contains_range(t.text_range()) && !t.kind().is_trivia())
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::LParen | SyntaxKind::RParen | SyntaxKind::Dot | SyntaxKind::Comma
            )
        })
        .collect();
    let last = tokens.last()?.text_range();
    match kind {
        Some(Kind::Module) => Some(TextRange::new(tokens[0].text_range().start(), last.end())),
        _ => Some(last),
    }
}
