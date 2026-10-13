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
    if let Some((declared, _)) = type_variable(&module.root(), offset) {
        return Some(Location {
            path: module.path.clone(),
            range: declared,
        });
    }
    if let Some(found) = crate::fields::definition(snapshot, index, module, offset) {
        return Some(found);
    }
    if let Some(name) = module_at(&module.root(), offset) {
        let path = module_path(index, &name)?;
        let text = snapshot.read(&path).ok()?;
        let root = parse_module(&text).syntax();
        let header = root
            .children()
            .find(|c| c.kind() == SyntaxKind::ModuleHeader)?;
        return Some(Location {
            path,
            range: crate::symbols::code_range(&header)?,
        });
    }
    let Some(target) = target_at(index, module, offset) else {
        // Where canonicalization stopped short, a local by its syntax.
        let (binder, _) = crate::scopes::local(&module.root(), offset)?;
        return Some(Location {
            path: module.path.clone(),
            range: declaring_node(&module.root(), binder).unwrap_or(binder),
        });
    };
    let found = declaration(snapshot, index, module, &target)?;
    // As elm-language-server answers: the node that declares the name.
    let text = snapshot.read(&found.path).ok()?;
    let root = parse_module(&text).syntax();
    // A pattern can't match on a record alias's constructor.
    let in_pattern = (module.root().token_at_offset(offset).right_biased()).is_some_and(|t| {
        t.parent_ancestors()
            .any(|a| a.kind() == SyntaxKind::ConstructorPattern)
    });
    let declares_alias = (root.token_at_offset(found.range.start()).right_biased())
        .and_then(|t| t.parent())
        .is_some_and(|p| p.kind() == SyntaxKind::TypeAliasDecl);
    if in_pattern && declares_alias {
        return None;
    }
    // An operator is its infix declaration's function.
    let found = match kind(&target) {
        Some(Kind::Binop) => infix_function(&root, found.range).map_or_else(
            || found.clone(),
            |range| Location {
                path: found.path.clone(),
                range,
            },
        ),
        _ => found,
    };
    Some(Location {
        range: declaring_node(&root, found.range).unwrap_or(found.range),
        path: found.path,
    })
}

/// The name of the function the infix declaration at `range` gives an
/// operator, where that file declares it.
fn infix_function(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let token = root.token_at_offset(range.start()).right_biased()?;
    let infix = token
        .parent_ancestors()
        .find(|a| a.kind() == SyntaxKind::InfixDecl)?;
    let function = (infix.children_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::LowerIdent)
        .last()?;
    let definition = (root.descendants())
        .filter(|n| n.kind() == SyntaxKind::Definition)
        .find(|d| {
            crate::lint::first_token(d, SyntaxKind::LowerIdent)
                .is_some_and(|t| t.text() == function.text())
        })?;
    crate::lint::first_token(&definition, SyntaxKind::LowerIdent).map(|t| t.text_range())
}

/// The range of the node that declares the name at `range`, as
/// tree-sitter-elm's are: a value's name and parameters, a whole type,
/// alias, port or module header, a variant, a record field, or a
/// variable itself; none past a doc comment.
fn declaring_node(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let token = root.token_at_offset(range.start()).right_biased()?;
    let parent = token.parent()?;
    let node = match parent.kind() {
        SyntaxKind::ModuleName => parent
            .parent()
            .filter(|p| p.kind() == SyntaxKind::ModuleHeader)?,
        _ => parent,
    };
    match node.kind() {
        SyntaxKind::Definition => {
            let end = (node.children())
                .filter(|c| ast::Pattern::can_cast(c.kind()))
                .last()
                .map_or_else(|| token.text_range().end(), |last| last.text_range().end());
            Some(TextRange::new(token.text_range().start(), end))
        }
        SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl => {
            crate::symbols::declaration_range(&node)
        }
        SyntaxKind::Variant => crate::symbols::variant_range(&node),
        SyntaxKind::PortDecl | SyntaxKind::FieldType | SyntaxKind::ModuleHeader => {
            crate::symbols::code_range(&node)
        }
        _ => None,
    }
}

/// Where the type of the expression at `offset` in the file at `path`
/// is declared: its custom type, or the alias it's written as; none for
/// a record, a function or a variable.
pub fn type_definition(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Option<Location> {
    use nomi_compiler::types::error;
    let module = index.module(path)?;
    let (_, types) = module.types(index.compat)?;
    let (innermost, ty) = (types.iter())
        .filter_map(|(range, ty)| Some((crate::hover::trimmed(&module.root(), *range)?, ty)))
        .filter(|(range, _)| range.contains_inclusive(offset))
        .min_by_key(|(range, _)| range.len())?;
    // A variable's context may have taken its alias; its binding keeps it.
    let named = |ty: &error::Type| matches!(ty, error::Type::Alias(..) | error::Type::Type(..));
    let bound = (!named(ty))
        .then(|| {
            let binder = (module.references.iter())
                .find(|r| r.range == innermost)
                .and_then(|r| match r.target {
                    Target::Local(binder) => Some(binder),
                    Target::Global { .. } => None,
                })?;
            (types.iter())
                .filter(|(range, _)| crate::hover::trimmed(&module.root(), *range) == Some(binder))
                .map(|(_, ty)| ty)
                .find(|ty| named(ty))
        })
        .flatten();
    let ty = bound.unwrap_or(ty);
    let (home, name, alias) = match ty {
        error::Type::Alias(home, name, _, _) => (home, name, true),
        error::Type::Type(home, name, _) => (home, name, false),
        _ => return None,
    };
    let path = module_path(index, &home.module)?;
    let text = snapshot.read(&path).ok()?;
    let root = parse_module(&text).syntax();
    let kind = if alias {
        SyntaxKind::TypeAliasDecl
    } else {
        SyntaxKind::CustomTypeDecl
    };
    let declaration = (root.children()).filter(|n| n.kind() == kind).find(|n| {
        crate::symbols::token(n, SyntaxKind::UpperIdent).as_deref() == Some(name.as_str())
    })?;
    Some(Location {
        path,
        range: crate::symbols::code_range(&declaration)?,
    })
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
    // Only the project's own files, as elm-language-server keeps to the
    // writeable ones: a package's declaration and uses are left out. Not
    // for a field, whose references it never filters.
    if let Some(module) = index.module(path)
        && let Some(found) =
            crate::fields::references(snapshot, index, module, offset, with_declaration)
    {
        return found;
    }
    let mut found = all_references(snapshot, index, path, offset, with_declaration);
    found.retain(|location| index.module(&location.path).is_some());
    found
}

fn all_references(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
    with_declaration: bool,
) -> Vec<Location> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    if let Some(found) =
        crate::fields::references(snapshot, index, module, offset, with_declaration)
    {
        return found;
    }
    if let Some(name) = module_at(&module.root(), offset) {
        return module_references(index, &name, with_declaration);
    }
    if let Some((declared, all)) = type_variable(&module.root(), offset) {
        return (all.into_iter())
            .filter(|range| with_declaration || *range != declared)
            .map(|range| Location {
                path: module.path.clone(),
                range,
            })
            .collect();
    }
    let target = target_at(index, module, offset);
    // The `List` type, declared nowhere, is its module to
    // elm-language-server.
    if let Some(Target::Global {
        home,
        name,
        kind: Kind::Type,
    }) = &target
        && &*home.package == "elm/core"
        && &*home.module == "List"
        && name == "List"
    {
        return module_references(index, "List", with_declaration);
    }
    let Some(target) = target else {
        let Some((binder, scope)) = crate::scopes::local(&module.root(), offset) else {
            return Vec::new();
        };
        return (crate::scopes::uses(&module.root(), binder, &scope).into_iter())
            .filter(|range| with_declaration || *range != binder)
            .map(|range| Location {
                path: module.path.clone(),
                range,
            })
            .collect();
    };
    let mut found = Vec::new();
    if let Some(declared) = declaration(snapshot, index, module, &target) {
        // The annotation names the definition too.
        let annotation = (index.modules.iter())
            .find(|m| m.path == declared.path)
            .and_then(|m| annotation_of(&m.root(), declared.range));
        if let Some(range) = annotation {
            found.push(Location {
                path: declared.path.clone(),
                range,
            });
        }
        if with_declaration {
            found.push(declared);
        }
    }
    // A local is one module's.
    let modules: Vec<&Indexed> = match target {
        Target::Local(_) => vec![module],
        Target::Global { .. } => index.modules.iter().map(|m| &**m).collect(),
    };
    // A record alias is also its constructor.
    let constructor = match &target {
        Target::Global {
            home,
            name,
            kind: Kind::Type,
        } if home.package == index.package => (index.named(&home.module))
            .filter(|m| m.hir.aliases.iter().any(|a| a.name.text == **name))
            .map(|_| Target::Global {
                home: home.clone(),
                name: name.clone(),
                kind: Kind::Ctor,
            }),
        _ => None,
    };
    for m in modules {
        for target in std::iter::once(&target).chain(&constructor) {
            found.extend(uses(m, target).map(|range| Location {
                path: m.path.clone(),
                range,
            }));
            // Where canonicalization stopped short, its names as written.
            if m.canonical.is_none() {
                for range in written_uses(index, m, target) {
                    if !found.iter().any(|l| l.path == m.path && l.range == range) {
                        found.push(Location {
                            path: m.path.clone(),
                            range,
                        });
                    }
                }
            }
        }
    }
    found
}

/// The names in `module` that name `target` as written, qualified with
/// its module or one of its aliases, or exposed by an import of it.
fn written_uses(index: &Index, module: &Indexed, target: &Target) -> Vec<TextRange> {
    let index_module = |_: &Indexed, home: &Home| index.named(&home.module);
    let Target::Global { home, name, kind } = target else {
        return Vec::new();
    };
    let imports: Vec<SyntaxNode> = (module.root().children())
        .filter(|n| n.kind() == SyntaxKind::Import)
        .filter(|i| {
            i.children()
                .find(|c| c.kind() == SyntaxKind::ModuleName)
                .is_some_and(|n| crate::symbols::words(&n) == *home.module)
        })
        .collect();
    let names: Vec<String> = imports
        .iter()
        .flat_map(|i| {
            let alias = (i.children().find(|c| c.kind() == SyntaxKind::ImportAlias))
                .and_then(|a| crate::lint::first_token(&a, SyntaxKind::UpperIdent))
                .map(|t| t.text().to_owned());
            alias
                .into_iter()
                .chain(std::iter::once(home.module.to_string()))
        })
        .collect();
    // A constructor is exposed with its type's `(..)`.
    let union = (*kind == Kind::Ctor)
        .then(|| {
            let declaring = index_module(module, home)?;
            let union = (declaring.hir.unions.iter())
                .find(|u| u.variants.iter().any(|(v, _)| v.text == **name))?;
            Some(format!("{}(..)", union.name.text))
        })
        .flatten();
    let exposed = imports.iter().any(|i| {
        i.children()
            .find(|c| c.kind() == SyntaxKind::ExposingList)
            .is_some_and(|list| {
                let written = list.text().to_string();
                crate::lint::first_token(&list, SyntaxKind::DotDot).is_some()
                    || list
                        .children()
                        .any(|item| crate::symbols::words(&item) == **name)
                    || union
                        .as_ref()
                        .is_some_and(|u| written.replace(' ', "").contains(u.as_str()))
            })
    });
    let mut found = Vec::new();
    for node in module.root().descendants() {
        let fits = match node.kind() {
            SyntaxKind::NameExpr => match crate::lint::first_token(&node, SyntaxKind::UpperIdent) {
                Some(_) => *kind == Kind::Ctor,
                None => *kind == Kind::Value,
            },
            SyntaxKind::ConstructorPattern => *kind == Kind::Ctor,
            SyntaxKind::NamedType => *kind == Kind::Type,
            _ => false,
        };
        if !fits {
            continue;
        }
        let written = (node.children_with_tokens())
            .filter_map(|e| e.into_token())
            .find(|t| matches!(t.kind(), SyntaxKind::UpperIdent | SyntaxKind::LowerIdent));
        let Some(written) = written.filter(|t| t.text() == &**name) else {
            continue;
        };
        let qualifier = (node.children().find(|c| c.kind() == SyntaxKind::Qualifier))
            .map(|q| crate::symbols::words(&q).trim_end_matches('.').to_owned());
        let named = match &qualifier {
            Some(q) => names.contains(q),
            None => exposed,
        };
        // A value's name alone, as tree-sitter-elm writes it.
        let range = match kind {
            Kind::Value => Some(written.text_range()),
            _ => qualified_range(&node),
        };
        if named && let Some(range) = range {
            found.push(range);
        }
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
        .filter_map(|r| {
            // tree-sitter-elm writes a capitalized name with its
            // qualifier as one node, and a value's name alone.
            let whole = matches!(kind(&r.target), Some(Kind::Type | Kind::Ctor));
            let kind = if whole {
                Some(Kind::Module)
            } else {
                kind(&r.target)
            };
            name_range(&module.root(), r.range, kind)
        })
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
    // An annotation's name is its definition's.
    if let Some(name) = annotated(&module.root(), offset) {
        return target_at(index, module, name.start());
    }
    // So is an infix declaration's function.
    let infix = (module.hir.binops.iter()).find(|b| b.function.range.contains_inclusive(offset));
    if let Some(infix) = infix {
        return Some(Target::Global {
            home: Home {
                package: index.package.clone().into(),
                module: module.name.clone().into(),
            },
            name: infix.function.text.clone(),
            kind: Kind::Value,
        });
    }
    let at = |range: Option<TextRange>| range.is_some_and(|r| r.contains_inclusive(offset));
    let used = (module.references.iter())
        .find(|r| at(name_range(&module.root(), r.range, kind(&r.target))));
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
        Target::Local(binder) if at(name_range(&module.root(), binder, None)) => {
            Some(r.target.clone())
        }
        _ => None,
    })
}

/// The module named at `offset`: in a header, an import, or a name's
/// qualifier, an import's alias, a default import's too, standing for
/// its module.
pub(crate) fn module_at(root: &SyntaxNode, offset: TextSize) -> Option<String> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::UpperIdent)?;
    let parent = token.parent()?;
    match parent.kind() {
        // An import or a header names its whole module.
        SyntaxKind::ModuleName => Some(crate::symbols::words(&parent)),
        SyntaxKind::Qualifier => {
            // tree-sitter-elm reads a qualified type or constructor as one
            // name, which the qualifier is part of, and a value's as its
            // whole module.
            let owner = parent.parent()?;
            let names_value = owner.kind() == SyntaxKind::NameExpr
                && crate::lint::first_token(&owner, SyntaxKind::LowerIdent).is_some();
            if !names_value {
                return None;
            }
            let written = crate::symbols::words(&parent)
                .trim_end_matches('.')
                .to_owned();
            let aliased = (root.children())
                .filter(|n| n.kind() == SyntaxKind::Import)
                .find_map(|import| {
                    let alias = import
                        .children()
                        .find(|c| c.kind() == SyntaxKind::ImportAlias)?;
                    let alias = crate::lint::first_token(&alias, SyntaxKind::UpperIdent)?;
                    let name = import
                        .children()
                        .find(|c| c.kind() == SyntaxKind::ModuleName)?;
                    (alias.text() == written).then(|| crate::symbols::words(&name))
                });
            // The default imports alias `Platform.Cmd` and `Platform.Sub`.
            let default = || {
                (crate::completion::default_imports().into_iter())
                    .find(|(_, alias)| alias.as_deref() == Some(written.as_str()))
                    .map(|(name, _)| name)
            };
            Some(aliased.or_else(default).unwrap_or(written))
        }
        _ => None,
    }
}

/// The file of the module `name`: the project's, or a dependency's.
pub(crate) fn module_path(index: &Index, name: &str) -> Option<PathBuf> {
    if let Some(module) = index.named(name) {
        return Some(module.path.clone());
    }
    let mut relative = PathBuf::from("src");
    relative.extend(name.split('.'));
    relative.set_extension("elm");
    let mut packages: Vec<&PathBuf> = index.packages.values().collect();
    packages.sort();
    packages
        .into_iter()
        .map(|dir| dir.join(&relative))
        .find(|p| p.is_file())
}

/// Every place the project names the module `name`, as
/// elm-language-server finds them: its header where `with_declaration`,
/// each import of it, and each name qualified with it as written.
fn module_references(index: &Index, name: &str, with_declaration: bool) -> Vec<Location> {
    let mut found = Vec::new();
    for module in &index.modules {
        let at = |range| Location {
            path: module.path.clone(),
            range,
        };
        // Imported with an alias, even one of the same name, its
        // qualifiers are the alias's. The default imports alias
        // `Platform.Cmd` and `Platform.Sub` everywhere.
        let aliased = matches!(name, "Platform.Cmd" | "Platform.Sub")
            || (module.root().children())
                .filter(|n| n.kind() == SyntaxKind::Import)
                .any(|i| {
                    i.children().any(|c| c.kind() == SyntaxKind::ImportAlias)
                        && i.children()
                            .find(|c| c.kind() == SyntaxKind::ModuleName)
                            .is_some_and(|n| crate::symbols::words(&n) == name)
                });
        for node in module.root().descendants() {
            match node.kind() {
                SyntaxKind::ModuleName => {
                    let header = node
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::ModuleHeader);
                    if crate::symbols::words(&node) == name
                        && (with_declaration || !header)
                        && let Some(range) = crate::symbols::code_range(&node)
                    {
                        found.push(at(range));
                    }
                }
                SyntaxKind::NameExpr | SyntaxKind::NamedType | SyntaxKind::ConstructorPattern => {
                    let qualifier = node.children().find(|c| c.kind() == SyntaxKind::Qualifier);
                    let written = qualifier.map(|q| crate::symbols::words(&q));
                    if !aliased
                        && written.is_some_and(|w| w.trim_end_matches('.') == name)
                        && let Some(range) = qualified_range(&node)
                    {
                        found.push(at(range));
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// A qualified name's range, from its qualifier through its name.
fn qualified_range(node: &SyntaxNode) -> Option<TextRange> {
    let mut tokens = (node.children_with_tokens())
        .flat_map(|e| match e {
            SyntaxElement::Node(n) if n.kind() == SyntaxKind::Qualifier => n
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .collect::<Vec<_>>(),
            SyntaxElement::Token(t) => vec![t],
            SyntaxElement::Node(_) => Vec::new(),
        })
        .filter(|t| !t.kind().is_trivia());
    let first = tokens.next()?;
    let last = tokens
        .take_while(|t| {
            matches!(
                t.kind(),
                SyntaxKind::UpperIdent | SyntaxKind::LowerIdent | SyntaxKind::Dot
            )
        })
        .last()
        .unwrap_or_else(|| first.clone());
    Some(TextRange::new(
        first.text_range().start(),
        last.text_range().end(),
    ))
}

/// The type variable at `offset`: where it's declared, and everywhere it
/// is, as elm-language-server resolves them. A custom type's or alias's
/// variable is its parameter's. An annotation's is its first in the
/// outermost annotation around it that has it, so a `let`'s annotation
/// shares the variables of its declaration's.
fn type_variable(root: &SyntaxNode, offset: TextSize) -> Option<(TextRange, Vec<TextRange>)> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let parent = token.parent()?;
    if !matches!(
        parent.kind(),
        SyntaxKind::TypeVar | SyntaxKind::RecordExtension | SyntaxKind::TypeParam
    ) {
        return None;
    }
    let name = token.text().to_owned();
    let occurrences = |node: &SyntaxNode| -> Vec<TextRange> {
        (node.descendants_with_tokens())
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::LowerIdent && t.text() == name)
            .filter(|t| {
                t.parent().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        SyntaxKind::TypeVar | SyntaxKind::RecordExtension | SyntaxKind::TypeParam
                    )
                })
            })
            .map(|t| t.text_range())
            .collect()
    };
    let declaration = parent.ancestors().find(|a| {
        matches!(
            a.kind(),
            SyntaxKind::TypeAliasDecl
                | SyntaxKind::CustomTypeDecl
                | SyntaxKind::Signature
                | SyntaxKind::PortDecl
        )
    })?;
    match declaration.kind() {
        SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl => {
            let all = occurrences(&declaration);
            let param = (declaration.children())
                .filter(|c| c.kind() == SyntaxKind::TypeParam)
                .find(|p| p.text() == name.as_str())?;
            let declared = all
                .iter()
                .copied()
                .find(|r| param.text_range().contains_range(*r))?;
            Some((declared, all))
        }
        SyntaxKind::Signature => {
            // The annotations around it, innermost first, each with its
            // declaration.
            let outermost = (declaration.ancestors())
                .filter(|a| matches!(a.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue))
                .filter_map(|d| {
                    let signature = d.children().find(|c| c.kind() == SyntaxKind::Signature)?;
                    (!occurrences(&signature).is_empty()).then_some((signature, d))
                })
                .last()?;
            let (signature, scope) = outermost;
            let declared = *occurrences(&signature).first()?;
            let all = (scope.descendants())
                .filter(|n| n.kind() == SyntaxKind::Signature)
                .flat_map(|s| occurrences(&s))
                .collect();
            Some((declared, all))
        }
        _ => {
            let all = occurrences(&declaration);
            Some((*all.first()?, all))
        }
    }
}

/// The name of the definition whose annotation names it at `offset`.
fn annotated(root: &SyntaxNode, offset: TextSize) -> Option<TextRange> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let signature = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::Signature)?;
    let definition = signature
        .next_sibling()
        .filter(|n| n.kind() == SyntaxKind::Definition)?;
    let name = (definition.children_with_tokens())
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    Some(name.text_range())
}

/// The annotation's name above the definition named at `range`, if any.
fn annotation_of(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let token = root.token_at_offset(range.start()).right_biased()?;
    let definition = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::Definition)?;
    let signature = definition
        .prev_sibling()
        .filter(|n| n.kind() == SyntaxKind::Signature)?;
    let name = (signature.children_with_tokens())
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    Some(name.text_range())
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
                range: name_range(&module.root(), *binder, None)?,
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
    // `List`, which elm/core never declares, is its module.
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
        // A qualified type or constructor is one name to tree-sitter-elm,
        // qualifier and all; a value's qualifier is its module's.
        Some(Kind::Module | Kind::Type | Kind::Ctor) => {
            Some(TextRange::new(tokens[0].text_range().start(), last.end()))
        }
        _ => Some(last),
    }
}
