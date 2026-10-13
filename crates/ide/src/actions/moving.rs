//! Moving a top-level function to another module
//! (`moveRefactoringHandler.ts`): the function goes, with its signature
//! and doc comment, to the end of the destination, and every module that
//! uses it imports it from there.

use super::*;

/// A place a function can move to: one of the project's other modules,
/// by its file's name and folder, as written from the project's root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub name: String,
    pub folder: String,
    pub path: PathBuf,
}

/// An edit of a move: a usual one, or text added at a line, which may be
/// past the destination's end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveEdit {
    Edit(Edit),
    AtLine(u32, String),
}

/// `getMoveDestinations`: the project's modules but the one at `source`.
pub fn move_destinations(index: &Index, root: &Path, source: &Path) -> Vec<Destination> {
    let source = index.module(source).map(|m| m.path.clone());
    let mut found: Vec<Destination> = (index.modules.iter())
        .filter(|m| Some(&m.path) != source.as_ref())
        .filter_map(|m| {
            let relative = m.path.strip_prefix(root).ok()?;
            Some(Destination {
                name: relative.file_name()?.to_string_lossy().into_owned(),
                folder: relative.parent()?.to_string_lossy().into_owned(),
                path: m.path.clone(),
            })
        })
        .collect();
    found.sort_by(|a, b| (&a.folder, &a.name).cmp(&(&b.folder, &b.name)));
    found
}

/// How a reference to the moved function is written.
struct Use {
    path: PathBuf,
    /// The qualifier's first name, where the reference is qualified.
    qualifier: Option<TextRange>,
    /// Whether it's the function's own signature, definition or an
    /// exposing list's entry.
    declaring: bool,
}

/// `handleMoveRequest`: each file's edits that move the function named
/// at `offset` in `source` to `destination`.
pub fn move_function(
    snapshot: &Snapshot,
    index: &Index,
    source: &Path,
    offset: TextSize,
    destination: &Path,
) -> Option<Vec<(PathBuf, Vec<MoveEdit>)>> {
    let module = index.module(source)?;
    let target = index.module(destination)?;
    let (root, text) = (&module.root(), &*module.text);
    let token = token_at(root, offset)?;
    let parent = token.parent()?;
    let declaration = match parent.kind() {
        SyntaxKind::Definition | SyntaxKind::Signature => parent.parent()?,
        _ => return None,
    };
    if declaration.kind() != SyntaxKind::ValueDecl {
        return None;
    }
    let definition = child(&declaration, SyntaxKind::Definition)?;
    let signature = child(&declaration, SyntaxKind::Signature);
    let name = token.text().to_owned();
    let first = signature.clone().unwrap_or_else(|| definition.clone());
    let comment = std::iter::successors(first.first_token()?.prev_token(), |t| t.prev_token())
        .find(|t| !matches!(t.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .filter(|t| matches!(t.kind(), SyntaxKind::BlockComment | SyntaxKind::DocComment));
    let written = |node: &SyntaxNode| {
        code_range(node).map(|r| text[std::ops::Range::<usize>::from(r)].to_owned())
    };
    let start = match (&comment, &signature) {
        (Some(comment), _) => comment.text_range().start(),
        (None, Some(signature)) => code_range(signature)?.start(),
        (None, None) => code_range(&definition)?.start(),
    };
    let end = code_range(&definition)?.end();
    let mut function = String::from("\n\n");
    if let Some(comment) = &comment {
        function.push_str(comment.text());
        function.push('\n');
    }
    if let Some(signature) = &signature {
        function.push_str(&written(signature)?);
        function.push('\n');
    }
    function.push_str(&written(&definition)?);
    let module_name = module.name.clone();
    let target_name = target.name.clone();

    let uses: Vec<Use> =
        (crate::references(snapshot, index, source, token.text_range().start(), true).into_iter())
            .filter_map(|location| {
                let other = index.module(&location.path)?;
                let at = token_at(&other.root(), location.range.end() - TextSize::new(1))?;
                let declaring = at.parent_ancestors().any(|a| {
                    matches!(a.kind(), SyntaxKind::ExposingList)
                        || (other.path == module.path
                            && (a == definition || signature.as_ref() == Some(&a)))
                });
                let qualifier = (at.parent())
                    .and_then(|p| p.children().find(|c| c.kind() == SyntaxKind::Qualifier))
                    .and_then(|q| crate::lint::first_token(&q, SyntaxKind::UpperIdent))
                    .map(|t| t.text_range());
                Some(Use {
                    path: other.path.clone(),
                    qualifier,
                    declaring,
                })
            })
            .collect();
    let source_uses = (uses.iter()).any(|u| u.path == module.path && !u.declaring);
    let mut files: Vec<(PathBuf, Vec<MoveEdit>)> = vec![
        (module.path.clone(), Vec::new()),
        (target.path.clone(), Vec::new()),
    ];
    let push = |files: &mut Vec<(PathBuf, Vec<MoveEdit>)>, path: &Path, edit: MoveEdit| match files
        .iter_mut()
        .find(|(p, _)| p == path)
    {
        Some((_, edits)) => edits.push(edit),
        None => files.push((path.to_owned(), vec![edit])),
    };
    push(
        &mut files,
        &module.path,
        MoveEdit::Edit(Edit {
            range: TextRange::new(start, end),
            text: String::new(),
        }),
    );
    let target_line = target.text.matches('\n').count() as u32 + 1;
    push(
        &mut files,
        &target.path,
        MoveEdit::AtLine(target_line, function),
    );
    if let Some(edit) = unexpose_edit(root, text, &name) {
        push(&mut files, &module.path, MoveEdit::Edit(edit));
    }
    // Each file that uses it: its import of the old module loses it.
    let mut using: Vec<PathBuf> = Vec::new();
    for u in &uses {
        if !using.contains(&u.path) {
            using.push(u.path.clone());
        }
    }
    for path in &using {
        let Some(other) = index.module(path) else {
            continue;
        };
        let import = (other.root().children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .find(|i| {
                child(i, SyntaxKind::ModuleName)
                    .is_some_and(|n| crate::symbols::words(&n) == module_name)
            });
        if let Some(import) = import {
            let list = child(&import, SyntaxKind::ExposingList);
            let exposed: Vec<SyntaxNode> = (list.iter())
                .flat_map(|l| l.children())
                .filter(|n| matches!(n.kind(), SyntaxKind::ExposedValue | SyntaxKind::ExposedType))
                .collect();
            if let Some(edit) =
                remove_from_import(&other.text, list.as_ref(), &exposed, &name, false)
            {
                push(&mut files, path, MoveEdit::Edit(edit));
            }
        }
    }
    // The destination exposes it where others use it unqualified.
    if uses
        .iter()
        .any(|u| u.path != target.path && u.qualifier.is_none())
        && let Some(edit) = expose_edit(&target.root(), &target.text, &name, false)
    {
        push(&mut files, &target.path, MoveEdit::Edit(edit));
    }
    // Qualified uses name the new module, or none in the destination.
    for u in &uses {
        let Some(qualifier) = u.qualifier else {
            continue;
        };
        let edit = if u.path == target.path {
            Edit {
                range: TextRange::new(qualifier.start(), qualifier.end() + TextSize::new(1)),
                text: String::new(),
            }
        } else {
            Edit {
                range: qualifier,
                text: target_name.clone(),
            }
        };
        push(&mut files, &u.path, MoveEdit::Edit(edit));
    }
    // And each file that uses it imports the new module.
    for path in using.iter().filter(|p| **p != target.path) {
        if *path == module.path && !source_uses {
            continue;
        }
        let Some(other) = index.module(path) else {
            continue;
        };
        let expose = uses
            .iter()
            .any(|u| u.path == *path && u.qualifier.is_none());
        let context = Context::new(snapshot, index, other, path);
        let edit = add_import(&context, &target_name, expose.then_some(name.as_str()), "");
        push(&mut files, path, MoveEdit::Edit(edit));
    }
    Some(files)
}
