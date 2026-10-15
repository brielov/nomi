//! Go to definition, find references and document highlights, from the
//! references canonicalization records for each module of the project,
//! in the index of the project that owns the file
//! (`nomi_engine::index`). A name's range is its own token's: `x` in
//! `A.x`, `+` in `(+)`. A local variable is found from its uses, so one
//! never used has nothing to show.

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_compiler::canonical::Home;
use nomi_compiler::canonicalize::references::{Kind, Target};
use nomi_compiler::hir;
use nomi_engine::Snapshot;
use nomi_engine::graph::module_path;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, child_token, parse_module,
    significant_tokens, tokens,
};

use crate::completion::DEFAULT_IMPORTS;
use crate::hover::{trimmed, types_at};
use crate::tree::{
    child, code_range, declaration_range, declared_type, last_child_token, variant_range, words,
    written_imports,
};

/// A range in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub range: TextRange,
}

/// Whether the references of a name take in its declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declaration {
    Include,
    Exclude,
}

/// Where the name at `offset` in the file at `path` is defined.
pub fn definition(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Option<Location> {
    defined(snapshot, index, path, offset).map(|(location, _)| location)
}

/// [`definition`], and the tree of the file it's in.
pub(crate) fn defined(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Option<(Location, SyntaxNode)> {
    let module = index.module(path)?;
    let root = module.root();
    let here = |range| Location {
        path: module.path.clone(),
        range,
    };
    if let Some((declared, _)) = type_variable(&root, offset) {
        return Some((here(declared), root));
    }
    if let Some(found) = crate::fields::definition(snapshot, index, module, offset) {
        return Some(found);
    }
    if let Some(name) = module_at(&root, offset) {
        let (path, root) = module_tree(snapshot, index, &name)?;
        let header = child(&root, SyntaxKind::ModuleHeader)?;
        let range = code_range(&header)?;
        return Some((Location { path, range }, root));
    }
    let Some(target) = target_at(index, module, offset) else {
        // Where canonicalization stopped short, a local by its syntax.
        let (binder, _) = crate::scopes::local(&root, offset)?;
        let range = declaring_node(&root, binder).unwrap_or(binder);
        return Some((here(range), root));
    };
    let (found, declaring) = definition_of(snapshot, index, module, &target)?;
    // A pattern can't match on a record alias's constructor.
    let in_pattern = (root.token_at_offset(offset).right_biased()).is_some_and(|t| {
        t.parent_ancestors()
            .any(|a| a.kind() == SyntaxKind::ConstructorPattern)
    });
    let declares_alias = (declaring
        .token_at_offset(found.range.start())
        .right_biased())
    .and_then(|t| t.parent())
    .is_some_and(|p| p.kind() == SyntaxKind::TypeAliasDecl);
    if in_pattern && declares_alias {
        return None;
    }
    // An operator is its infix declaration's function.
    let range = matches!(
        target,
        Target::Global {
            kind: Kind::Binop,
            ..
        }
    )
    .then(|| infix_function(&declaring, found.range))
    .flatten()
    .unwrap_or(found.range);
    // As elm-language-server answers: the node that declares the name.
    let range = declaring_node(&declaring, range).unwrap_or(range);
    Some((
        Location {
            path: found.path,
            range,
        },
        declaring,
    ))
}

/// The name of the function the infix declaration at `range` gives an
/// operator, where that file declares it.
fn infix_function(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let token = root.token_at_offset(range.start()).right_biased()?;
    let infix = token
        .parent_ancestors()
        .find(|a| a.kind() == SyntaxKind::InfixDecl)?;
    let function = last_child_token(&infix, SyntaxKind::LowerIdent)?;
    (root.descendants())
        .filter(|n| n.kind() == SyntaxKind::Definition)
        .find_map(|d| {
            child_token(&d, SyntaxKind::LowerIdent).filter(|t| t.text() == function.text())
        })
        .map(|t| t.text_range())
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
        SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl => declaration_range(&node),
        SyntaxKind::Variant => variant_range(&node),
        SyntaxKind::PortDecl | SyntaxKind::FieldType | SyntaxKind::ModuleHeader => {
            code_range(&node)
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
    type_defined(snapshot, index, path, offset).map(|(location, _)| location)
}

/// [`type_definition`], and the tree of the file it's in.
pub(crate) fn type_defined(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Option<(Location, SyntaxNode)> {
    use nomi_compiler::types::error;
    let module = index.module(path)?;
    let root = module.root();
    let (_, types) = module.types(index.compat)?;
    let (innermost, ty) = (types.iter())
        .filter(|(range, _)| range.contains_inclusive(offset))
        .filter_map(|(range, ty)| Some((trimmed(&root, *range)?, ty)))
        .filter(|(range, _)| range.contains_inclusive(offset))
        .min_by_key(|(range, _)| range.len())?;
    // A variable's context may have taken its alias; its binding keeps it.
    let named =
        |ty: &error::Type| matches!(ty, error::Type::Alias { .. } | error::Type::Type { .. });
    let bound = (!named(ty))
        .then(|| {
            let binder = (module.references.iter())
                .find(|r| r.range == innermost)
                .and_then(|r| match r.target {
                    Target::Local(binder) => Some(binder),
                    Target::Global { .. } => None,
                })?;
            types_at(&root, types, binder).find(|ty| named(ty))
        })
        .flatten();
    let (home, name, kind) = match bound.unwrap_or(ty) {
        error::Type::Alias { home, name, .. } => (home, name, SyntaxKind::TypeAliasDecl),
        error::Type::Type { home, name, .. } => (home, name, SyntaxKind::CustomTypeDecl),
        _ => return None,
    };
    let (path, root) = module_tree(snapshot, index, &home.module)?;
    let range = code_range(&declared_type(&root, kind, name)?)?;
    Some((Location { path, range }, root))
}

/// Every use of the name at `offset` in the file at `path`, across the
/// project, in the order of the project's modules and then of the text;
/// its definition first where `declaration` says so.
pub fn references(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
    declaration: Declaration,
) -> Vec<Location> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    // Only the project's own files, as elm-language-server keeps to the
    // writeable ones: a package's declaration and uses are left out. Not
    // for a field, whose references it never filters.
    if let Some(found) = crate::fields::references(snapshot, index, module, offset, declaration) {
        return found;
    }
    let mut found = all_references(snapshot, index, module, offset, declaration);
    found.retain(|location| index.module(&location.path).is_some());
    found
}

fn all_references(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
    declaration: Declaration,
) -> Vec<Location> {
    let root = module.root();
    let here = |ranges: Vec<TextRange>, declared: TextRange| -> Vec<Location> {
        (ranges.into_iter())
            .filter(|range| declaration == Declaration::Include || *range != declared)
            .map(|range| Location {
                path: module.path.clone(),
                range,
            })
            .collect()
    };
    if let Some(name) = module_at(&root, offset) {
        return module_references(index, &name, declaration);
    }
    if let Some((declared, all)) = type_variable(&root, offset) {
        return here(all, declared);
    }
    let target = target_at(index, module, offset);
    // The `List` type, declared nowhere, is its module to
    // elm-language-server.
    if let Some(Target::Global {
        home,
        name,
        kind: Kind::Type,
    }) = &target
        && home.is("elm/core", "List")
        && name == "List"
    {
        return module_references(index, "List", declaration);
    }
    match target {
        Some(target) => target_references(snapshot, index, module, &target, declaration),
        None => match crate::scopes::local(&root, offset) {
            Some((binder, scope)) => here(crate::scopes::uses(&root, binder, &scope), binder),
            None => Vec::new(),
        },
    }
}

/// Every use of `target`, which a name in `module` refers to: in
/// `module` for a local, else in each of the project's modules, after
/// its annotation and declaration.
fn target_references(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    target: &Target,
    declaration: Declaration,
) -> Vec<Location> {
    let mut found = Vec::new();
    if let Some((declared, root)) = definition_of(snapshot, index, module, target) {
        // The annotation names the definition too.
        let own = index.modules.iter().any(|m| m.path == declared.path);
        if let Some(range) = own.then(|| annotation_of(&root, declared.range)).flatten() {
            found.push(Location {
                path: declared.path.clone(),
                range,
            });
        }
        if declaration == Declaration::Include {
            found.push(declared);
        }
    }
    // A local is one module's.
    let modules: Vec<&Indexed> = match target {
        Target::Local(_) => vec![module],
        Target::Global { .. } => index.modules.iter().map(|m| &**m).collect(),
    };
    // A record alias is also its constructor.
    let constructor = match target {
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
        let at = |range| Location {
            path: m.path.clone(),
            range,
        };
        for target in std::iter::once(target).chain(&constructor) {
            found.extend(uses(m, target).map(at));
            // Where canonicalization stopped short, its names as written.
            if m.canonical.is_none() {
                for range in written_uses(index, m, target) {
                    if !found.iter().any(|l| l.path == m.path && l.range == range) {
                        found.push(at(range));
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
    let Target::Global { home, name, kind } = target else {
        return Vec::new();
    };
    let root = module.root();
    let imports: Vec<_> = written_imports(&root)
        .filter(|i| i.module == *home.module)
        .collect();
    let names: Vec<&str> = (imports.iter())
        .flat_map(|i| [i.alias.as_deref(), Some(&*home.module)])
        .flatten()
        .collect();
    // A constructor is exposed with its type's `(..)`.
    let union = (*kind == Kind::Ctor)
        .then(|| {
            let declaring = index.named(&home.module)?;
            let union = (declaring.hir.unions.iter())
                .find(|u| u.variants.iter().any(|(v, _)| v.text == **name))?;
            Some(format!("{}(..)", union.name.text))
        })
        .flatten();
    let exposed = imports.iter().any(|i| {
        i.node.exposing_list().is_some_and(|list| {
            let list = list.syntax();
            child_token(list, SyntaxKind::DotDot).is_some()
                || list.children().any(|item| words(&item) == **name)
                || union.as_ref().is_some_and(|u| {
                    let written = list.text().to_string();
                    written.replace(' ', "").contains(u.as_str())
                })
        })
    });
    let mut found = Vec::new();
    for node in root.descendants() {
        let fits = match node.kind() {
            SyntaxKind::NameExpr => match child_token(&node, SyntaxKind::UpperIdent) {
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
            .filter_map(SyntaxElement::into_token)
            .find(|t| matches!(t.kind(), SyntaxKind::UpperIdent | SyntaxKind::LowerIdent));
        let Some(written) = written.filter(|t| t.text() == &**name) else {
            continue;
        };
        let named = match child(&node, SyntaxKind::Qualifier) {
            Some(qualifier) => names.contains(&words(&qualifier).trim_end_matches('.')),
            None => exposed,
        };
        // A value's name alone, as tree-sitter-elm writes it.
        let range = match kind {
            Kind::Value => Some(written.text_range()),
            Kind::Type | Kind::Ctor | Kind::Binop | Kind::Module => qualified_range(&node),
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
    let mut found: Vec<TextRange> = (definition_of(snapshot, index, module, &target))
        .filter(|(d, _)| d.path == module.path)
        .map(|(d, _)| d.range)
        .into_iter()
        .collect();
    found.extend(uses(module, &target));
    found.sort_by_key(|r| r.start());
    found.dedup();
    found
}

/// The ranges in `module` of the names that refer to `target`.
fn uses<'m>(module: &'m Indexed, target: &'m Target) -> impl Iterator<Item = TextRange> + 'm {
    let root = module.root();
    let span = span(target);
    (module.references.iter())
        .filter(move |r| r.target == *target)
        .filter_map(move |r| name_range(&root, r.range, span))
}

/// What the name at `offset` refers to, or names where it is defined:
/// a reference, a top-level declaration, or a local's binding.
fn target_at(index: &Index, module: &Indexed, offset: TextSize) -> Option<Target> {
    let root = module.root();
    // An annotation's name is its definition's.
    if let Some(name) = annotated(&root, offset) {
        return target_at(index, module, name.start());
    }
    let home = || Home {
        package: index.package.as_str().into(),
        module: module.name.as_str().into(),
    };
    // So is an infix declaration's function.
    let infix = (module.hir.binops.iter()).find(|b| b.function.range.contains_inclusive(offset));
    if let Some(infix) = infix {
        return Some(Target::Global {
            home: home(),
            name: infix.function.text.clone(),
            kind: Kind::Value,
        });
    }
    // A name's range is within its reference's.
    let at = |range: TextRange, span| {
        range.contains_inclusive(offset)
            && name_range(&root, range, span).is_some_and(|r| r.contains_inclusive(offset))
    };
    let used = (module.references.iter()).find(|r| at(r.range, span(&r.target)));
    if let Some(reference) = used {
        return Some(reference.target.clone());
    }
    let declared =
        declarations(&module.hir).find(|(name, _)| name.range.contains_inclusive(offset));
    if let Some((name, kind)) = declared {
        return Some(Target::Global {
            home: home(),
            name: name.text.clone(),
            kind,
        });
    }
    (module.references.iter()).find_map(|r| match r.target {
        Target::Local(binder) if at(binder, NameSpan::Last) => Some(r.target.clone()),
        _ => None,
    })
}

/// The module named at `offset`: in a header, an import, or a name's
/// qualifier, an import's alias, a default import's too, standing for
/// its module.
fn module_at(root: &SyntaxNode, offset: TextSize) -> Option<String> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::UpperIdent)?;
    let parent = token.parent()?;
    match parent.kind() {
        // An import or a header names its whole module.
        SyntaxKind::ModuleName => Some(words(&parent)),
        SyntaxKind::Qualifier => {
            // tree-sitter-elm reads a qualified type or constructor as one
            // name, which the qualifier is part of, and a value's as its
            // whole module.
            let owner = parent.parent()?;
            let names_value = owner.kind() == SyntaxKind::NameExpr
                && child_token(&owner, SyntaxKind::LowerIdent).is_some();
            if !names_value {
                return None;
            }
            let written = words(&parent).trim_end_matches('.').to_owned();
            let aliased = written_imports(root)
                .find(|i| i.alias.as_deref() == Some(written.as_str()))
                .map(|i| i.module);
            // The default imports alias `Platform.Cmd` and `Platform.Sub`.
            let default = || {
                (DEFAULT_IMPORTS.iter())
                    .find(|d| d.alias == Some(written.as_str()))
                    .map(|d| d.module.to_owned())
            };
            Some(aliased.or_else(default).unwrap_or(written))
        }
        _ => None,
    }
}

/// The file of the module `name`: the project's, or a dependency's.
pub(crate) fn module_file(snapshot: &Snapshot, index: &Index, name: &str) -> Option<PathBuf> {
    if let Some(module) = index.named(name) {
        return Some(module.path.clone());
    }
    let relative = Path::new("src").join(module_path(name, "elm"));
    let mut packages: Vec<&PathBuf> = index.packages.values().collect();
    packages.sort();
    (packages.into_iter())
        .map(|dir| dir.join(&relative))
        .find(|p| snapshot.is_file(p))
}

/// The file of the module `name` and its tree: the project's, as
/// indexed, or a dependency's.
pub(crate) fn module_tree(
    snapshot: &Snapshot,
    index: &Index,
    name: &str,
) -> Option<(PathBuf, SyntaxNode)> {
    if let Some(module) = index.named(name) {
        return Some((module.path.clone(), module.root()));
    }
    let path = module_file(snapshot, index, name)?;
    let root = parse_module(&snapshot.read(&path).ok()?).syntax();
    Some((path, root))
}

/// Every place the project names the module `name`, as
/// elm-language-server finds them: its header where `declaration` says
/// so, each import of it, and each name qualified with it as written.
fn module_references(index: &Index, name: &str, declaration: Declaration) -> Vec<Location> {
    let mut found = Vec::new();
    for module in &index.modules {
        let root = module.root();
        let at = |range| Location {
            path: module.path.clone(),
            range,
        };
        // Imported with an alias, even one of the same name, its
        // qualifiers are the alias's. The default imports alias
        // `Platform.Cmd` and `Platform.Sub` everywhere.
        let aliased = (DEFAULT_IMPORTS.iter()).any(|d| d.alias.is_some() && d.module == name)
            || written_imports(&root).any(|i| i.node.import_alias().is_some() && i.module == name);
        for node in root.descendants() {
            match node.kind() {
                SyntaxKind::ModuleName => {
                    let header = node
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::ModuleHeader);
                    if (declaration == Declaration::Include || !header)
                        && words(&node) == name
                        && let Some(range) = code_range(&node)
                    {
                        found.push(at(range));
                    }
                }
                SyntaxKind::NameExpr | SyntaxKind::NamedType | SyntaxKind::ConstructorPattern => {
                    let qualifier = child(&node, SyntaxKind::Qualifier);
                    if !aliased
                        && qualifier.is_some_and(|q| words(&q).trim_end_matches('.') == name)
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
    let mut names = (node.children_with_tokens())
        .flat_map(|e| {
            let qualifier = (e.as_node())
                .filter(|n| n.kind() == SyntaxKind::Qualifier)
                .map(tokens);
            qualifier.into_iter().flatten().chain(e.into_token())
        })
        .filter(|t| !t.kind().is_trivia());
    let first = names.next()?.text_range();
    let last = names
        .take_while(|t| {
            matches!(
                t.kind(),
                SyntaxKind::UpperIdent | SyntaxKind::LowerIdent | SyntaxKind::Dot
            )
        })
        .last()
        .map_or(first, |t| t.text_range());
    Some(first.cover(last))
}

/// The type variable at `offset`: where it's declared, and everywhere it
/// is, as elm-language-server resolves them. A custom type's or alias's
/// variable is its parameter's. An annotation's is its first in the
/// outermost annotation around it that has it, so a `let`'s annotation
/// shares the variables of its declaration's.
fn type_variable(root: &SyntaxNode, offset: TextSize) -> Option<(TextRange, Vec<TextRange>)> {
    let variable = |kind| {
        matches!(
            kind,
            SyntaxKind::TypeVar | SyntaxKind::RecordExtension | SyntaxKind::TypeParam
        )
    };
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let parent = token.parent().filter(|p| variable(p.kind()))?;
    let name = token.text();
    let occurrences = |node: &SyntaxNode| -> Vec<TextRange> {
        tokens(node)
            .filter(|t| t.kind() == SyntaxKind::LowerIdent && t.text() == name)
            .filter(|t| t.parent().is_some_and(|p| variable(p.kind())))
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
                .find(|p| p.text() == name)?;
            let declared = all
                .iter()
                .copied()
                .find(|r| param.text_range().contains_range(*r))?;
            Some((declared, all))
        }
        SyntaxKind::Signature => {
            // The annotations around it, innermost first, each with its
            // declaration.
            let (signature, scope) = (declaration.ancestors())
                .filter(|a| matches!(a.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue))
                .filter_map(|d| {
                    let signature = child(&d, SyntaxKind::Signature)?;
                    (!occurrences(&signature).is_empty()).then_some((signature, d))
                })
                .last()?;
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

/// The name of the definition right after `signature`, which it
/// annotates.
pub(crate) fn annotated_name(signature: &SyntaxNode) -> Option<SyntaxToken> {
    let definition = signature
        .next_sibling()
        .filter(|n| n.kind() == SyntaxKind::Definition)?;
    child_token(&definition, SyntaxKind::LowerIdent)
}

/// The name of the annotation right before `definition`.
pub(crate) fn annotation_name(definition: &SyntaxNode) -> Option<SyntaxToken> {
    let signature = definition
        .prev_sibling()
        .filter(|n| n.kind() == SyntaxKind::Signature)?;
    child_token(&signature, SyntaxKind::LowerIdent)
}

/// The name of the definition whose annotation names it at `offset`.
fn annotated(root: &SyntaxNode, offset: TextSize) -> Option<TextRange> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let signature = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::Signature)?;
    Some(annotated_name(&signature)?.text_range())
}

/// The annotation's name above the definition named at `range`, if any.
fn annotation_of(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let token = root.token_at_offset(range.start()).right_biased()?;
    let definition = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::Definition)?;
    Some(annotation_name(&definition)?.text_range())
}

/// Where `target`, which a name in `module` refers to, is defined, and
/// the tree of the file that defines it.
fn definition_of(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    target: &Target,
) -> Option<(Location, SyntaxNode)> {
    let (home, name, kind) = match target {
        Target::Local(binder) => {
            let root = module.root();
            let range = name_range(&root, *binder, NameSpan::Last)?;
            let path = module.path.clone();
            return Some((Location { path, range }, root));
        }
        Target::Global { home, name, kind } => (home, name, *kind),
    };
    if home.package == index.package {
        let declaring = index.named(&home.module)?;
        let range = declared(&declaring.hir, name, kind)?;
        let path = declaring.path.clone();
        return Some((Location { path, range }, declaring.root()));
    }
    // A dependency's module, read from its sources.
    let path = (index.packages.get(&*home.package)?)
        .join("src")
        .join(module_path(&home.module, "elm"));
    let root = parse_module(&snapshot.read(&path).ok()?).syntax();
    let hir = hir::lower(&ast::Module::cast(root.clone())?);
    // `List`, which elm/core never declares, is its module.
    let range = declared(&hir, name, kind).unwrap_or_default();
    Some((Location { path, range }, root))
}

/// The range of the name `name`, a `kind`, that `module` declares at the
/// top level; for a module, its name in the header.
fn declared(module: &hir::Module, name: &str, kind: Kind) -> Option<TextRange> {
    if kind == Kind::Module {
        return Some(module.name.as_ref().map(|n| n.range).unwrap_or_default());
    }
    // A record alias is its own constructor.
    let found = declarations(module).find(|(n, k)| n.text == name && *k == kind);
    let alias = || {
        (kind == Kind::Ctor)
            .then(|| module.aliases.iter().find(|a| a.name.text == name))
            .flatten()
            .map(|a| a.name.range)
    };
    found.map(|(n, _)| n.range).or_else(alias)
}

/// Each top-level name `module` declares, and its kind.
fn declarations(module: &hir::Module) -> impl Iterator<Item = (&hir::Spanned, Kind)> {
    let unions = module.unions.iter().flat_map(|u| {
        std::iter::once((&u.name, Kind::Type))
            .chain(u.variants.iter().map(|(v, _)| (v, Kind::Ctor)))
    });
    (module.value_names().map(|name| (name, Kind::Value)))
        .chain(unions)
        .chain(module.aliases.iter().map(|a| (&a.name, Kind::Type)))
        .chain(module.ports.iter().map(|p| (&p.name, Kind::Value)))
        .chain(module.binops.iter().map(|b| (&b.op, Kind::Binop)))
}

/// How much of a written name its range takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameSpan {
    /// Its qualifier and all, as tree-sitter-elm reads a type's, a
    /// constructor's or a module's.
    Whole,
    /// Its last part: a value's name alone.
    Last,
}

/// How much of a name that refers to `target` its range takes.
fn span(target: &Target) -> NameSpan {
    match target {
        Target::Global {
            kind: Kind::Type | Kind::Ctor | Kind::Module,
            ..
        } => NameSpan::Whole,
        Target::Global {
            kind: Kind::Value | Kind::Binop,
            ..
        }
        | Target::Local(_) => NameSpan::Last,
    }
}

/// The name in `range`, which may hold trivia before it and a qualifier
/// or parentheses around it, as much of it as `span` says.
fn name_range(root: &SyntaxNode, range: TextRange, span: NameSpan) -> Option<TextRange> {
    let range = range.intersect(root.text_range())?;
    let node = match root.covering_element(range) {
        SyntaxElement::Node(node) => node,
        SyntaxElement::Token(token) => token.parent()?,
    };
    let mut names = significant_tokens(&node)
        .filter(|t| range.contains_range(t.text_range()))
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::LParen | SyntaxKind::RParen | SyntaxKind::Dot | SyntaxKind::Comma
            )
        })
        .map(|t| t.text_range());
    let first = names.next()?;
    let last = names.last().unwrap_or(first);
    Some(match span {
        NameSpan::Whole => first.cover(last),
        NameSpan::Last => last,
    })
}
