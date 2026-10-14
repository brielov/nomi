//! The inferred annotation, as a fix and a refactor, and exposing or
//! unexposing a declaration.

use std::path::Path;

use nomi_base::TextRange;
use nomi_engine::Snapshot;
use nomi_engine::index::Index;
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use super::{
    ActionCommand, Diagnosed, Fixing, Moving, Offer, Refactor, RefactorAction, RenameAt,
    definition_name, extract, here, name_of, token_at,
};
use crate::completion::Context;
use crate::lens::{Exposable, exposes};
use crate::problems::Code;
use crate::tree::{child, code_range};
use crate::{Edit, column};

/// `addTypeAnnotationCodeAction`'s edit: the inferred annotation above the
/// definition whose name is at `range`, at its column.
fn annotation_edit(context: &Context<'_>, range: TextRange) -> Option<Edit> {
    let module = context.module;
    let token = token_at(&module.root(), range.start())?;
    let definition = definition_name(&token)?;
    let ty = crate::hover::definition_type(context.index, module, &definition)?;
    let start = token.text_range().start();
    let indent = " ".repeat(column(&module.text, start));
    Some(Edit::insert(
        start,
        format!("{} : {ty}\n{indent}", token.text()),
    ))
}

pub(super) fn annotation_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (annotation_edit(&fixing.context, range).into_iter())
        .map(|edit| Offer::new("Add inferred annotation", here(fixing, vec![edit])))
        .collect()
}

pub(super) fn annotation_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    if let Some(found) = annotation_edit(&fixing.context, diagnostic.range)
        && !edits.iter().any(|e| e.text == found.text)
    {
        edits.push(found);
    }
}

/// The refactors at `range`, in elm-language-server's registration
/// order.
pub(super) fn refactors(fixing: &Fixing<'_>, range: TextRange) -> Vec<Refactor> {
    let mut found = Vec::new();
    let root = &fixing.context.module.root();
    let text = &*fixing.context.module.text;
    let Some(token) = token_at(root, range.start()) else {
        return found;
    };
    // An unannotated value's inferred annotation, where no diagnostic
    // offers it already.
    let annotated = definition_name(&token)
        .and_then(|d| d.parent())
        .map(|d| child(&d, SyntaxKind::Signature).is_some());
    let given = fixing.request.given;
    if annotated == Some(false) && !given.iter().any(|d| d.is(Code::MissingTypeAnnotation)) {
        let action = RefactorAction::AddTypeAnnotation;
        found.push(Refactor::new(action, "Add inferred annotation"));
    }
    found.extend(extract::extract_refactors(fixing, range));
    let parent = token.parent();
    let parent_kind = parent.as_ref().map(SyntaxNode::kind);
    let named = parent.as_ref().and_then(name_of).as_ref() == Some(&token);
    let in_let = token
        .parent_ancestors()
        .any(|a| a.kind() == SyntaxKind::LetExpr);
    // A value's name, or what tree-sitter-elm takes for one: the whole
    // type of a signature, whose punctuation is its own.
    let function = if token.kind() == SyntaxKind::LowerIdent
        && named
        && matches!(
            parent_kind,
            Some(SyntaxKind::Signature | SyntaxKind::Definition)
        ) {
        Some(token.text().to_owned())
    } else {
        signature_type(&token).map(|ty| written(text, &ty).to_owned())
    };
    if let Some(function) = &function
        && !in_let
    {
        found.push(if exposes(root, function, Exposable::Value) {
            Refactor::new(RefactorAction::UnexposeFunction, "Unexpose Function")
        } else {
            Refactor::new(RefactorAction::ExposeFunction, "Expose Function")
        });
    }
    if token.kind() == SyntaxKind::UpperIdent
        && named
        && let Some(kind @ (SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl)) = parent_kind
    {
        let alias = if kind == SyntaxKind::TypeAliasDecl {
            " Alias"
        } else {
            ""
        };
        if exposes(root, token.text(), Exposable::Type) {
            let title = format!("Unexpose Type{alias}");
            found.push(Refactor::new(RefactorAction::UnexposeType, title));
        } else {
            let title = format!("Expose Type{alias}");
            found.push(Refactor::new(RefactorAction::ExposeType, title));
            if kind == SyntaxKind::CustomTypeDecl {
                let action = RefactorAction::ExposeTypeWithVariants;
                found.push(Refactor::new(action, "Expose Type with Variants"));
            }
        }
    }
    // Moving a function, which the client asks where to and the server
    // then does.
    if fixing.request.moving == Moving::Supported
        && let Some(function) = function
        && !in_let
    {
        found.push(Refactor {
            command: Some(ActionCommand::MoveFunction { name: function }),
            ..Refactor::new(RefactorAction::MoveFunction, "Move Function")
        });
    }
    found
}

/// The type of a signature, when `token` is punctuation tree-sitter-elm
/// gives to its outermost `type_expression`: the arrows of a function
/// type written there, and the parentheses around it or its parts.
fn signature_type(token: &SyntaxToken) -> Option<SyntaxNode> {
    if token.kind().is_trivia()
        || matches!(
            token.kind(),
            SyntaxKind::LowerIdent | SyntaxKind::UpperIdent
        )
    {
        return None;
    }
    // tree-sitter-elm's type_expression is flat where nomi nests a
    // function type's arrows to the right.
    let parent = token.parent()?;
    let top = match parent.kind() {
        SyntaxKind::FunctionType => outermost_function(parent),
        SyntaxKind::ParenType => match parent.parent()? {
            up if up.kind() == SyntaxKind::FunctionType => outermost_function(up),
            _ => parent,
        },
        _ => return None,
    };
    top.parent()
        .is_some_and(|s| s.kind() == SyntaxKind::Signature)
        .then_some(top)
}

/// The outermost of the function types `ty` is the result of, or `ty`.
pub(super) fn outermost_function(mut ty: SyntaxNode) -> SyntaxNode {
    while let Some(up) = (ty.parent()).filter(|u| u.kind() == SyntaxKind::FunctionType) {
        ty = up;
    }
    ty
}

/// A refactor's edits, as resolving it gives them.
pub fn refactor_edits(
    index: &Index,
    snapshot: &Snapshot,
    path: &Path,
    action: RefactorAction,
    range: TextRange,
) -> Option<(Vec<Edit>, Option<RenameAt>)> {
    let module = index.module(path)?;
    let context = Context::new(snapshot, index, module, path);
    refactor_edits_in(&context, action, range)
}

pub(super) fn refactor_edits_in(
    context: &Context<'_>,
    action: RefactorAction,
    range: TextRange,
) -> Option<(Vec<Edit>, Option<RenameAt>)> {
    let module = context.module;
    // What's exposed or unexposed: the name at the range.
    let name = || {
        let token = token_at(&module.root(), range.start())?;
        Some(match signature_type(&token) {
            Some(ty) => written(&module.text, &ty).to_owned(),
            None => token.text().to_owned(),
        })
    };
    let (root, text) = (&module.root(), &*module.text);
    let edit = match action {
        RefactorAction::AddTypeAnnotation => annotation_edit(context, range),
        RefactorAction::ExtractFunction(place) => {
            return extract::function_edits(context, place, range);
        }
        RefactorAction::ExtractTypeAlias(of) => {
            return extract::type_alias_edits(context, of, range);
        }
        RefactorAction::ExposeFunction | RefactorAction::ExposeType => {
            expose_edit(root, text, &name()?)
        }
        RefactorAction::ExposeTypeWithVariants => {
            expose_edit(root, text, &format!("{}(..)", name()?))
        }
        RefactorAction::UnexposeFunction | RefactorAction::UnexposeType => {
            unexpose_edit(root, text, &name()?)
        }
        RefactorAction::MoveFunction => None,
    };
    edit.map(|edit| (vec![edit], None))
}

/// The exposed values and types of the module's exposing list
/// (`getModuleExposingListNodes`).
fn exposed_nodes(root: &SyntaxNode) -> Vec<SyntaxNode> {
    (root.children())
        .find(|n| n.kind() == SyntaxKind::ModuleHeader)
        .and_then(|header| child(&header, SyntaxKind::ExposingList))
        .map(|list| {
            (list.descendants())
                .filter(|n| matches!(n.kind(), SyntaxKind::ExposedValue | SyntaxKind::ExposedType))
                .collect()
        })
        .unwrap_or_default()
}

/// `node` as `text` writes it, without the trivia around it.
pub(super) fn written<'t>(text: &'t str, node: &SyntaxNode) -> &'t str {
    code_range(node).map_or("", |r| &text[r])
}

/// Whether a name goes into a module's exposing list or out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    Expose,
    Unexpose,
}

/// The edit of the file at `path` that exposes `name`, or unexposes it:
/// elm-language-server's `elm/expose` and `elm/unexpose` requests.
pub fn exposing_edit(
    snapshot: &Snapshot,
    path: &Path,
    name: &str,
    exposure: Exposure,
) -> Option<Edit> {
    let text = snapshot.read(path).ok()?;
    let root = nomi_syntax::parse_module(&text).syntax();
    match exposure {
        Exposure::Expose => expose_edit(&root, &text, name),
        Exposure::Unexpose => unexpose_edit(&root, &text, name),
    }
}

/// `exposeValueInModule`: `, entry` after the last exposed, unless the
/// list is `..` or has it already.
pub(super) fn expose_edit(root: &SyntaxNode, text: &str, entry: &str) -> Option<Edit> {
    let nodes = exposed_nodes(root);
    if nodes.iter().any(|n| written(text, n) == entry) {
        return None;
    }
    let last = code_range(nodes.last()?)?;
    Some(Edit::insert(last.end(), format!(", {entry}")))
}

/// `unexposedValueInModule`: `name` out of the exposing list, with the
/// comma between it and its neighbor, unless it's the only one.
pub(super) fn unexpose_edit(root: &SyntaxNode, text: &str, name: &str) -> Option<Edit> {
    let nodes = exposed_nodes(root);
    if nodes.len() <= 1 {
        return None;
    }
    remove_exposed(&nodes, text, name, Commas::Nearest)
}

/// Which commas go with a name out of a list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Commas {
    /// The one after it, or the one before it where it's last.
    Nearest,
    /// The one before it too.
    Before,
    /// The one before it, or else the one after it.
    BeforeElseAfter,
}

/// `removeValueFromExposingList`: the exposed `name` (or `name(..)`) out
/// of its list, with the commas `commas` says.
pub(super) fn remove_exposed(
    nodes: &[SyntaxNode],
    text: &str,
    name: &str,
    commas: Commas,
) -> Option<Edit> {
    let node = (nodes.iter()).find(|n| {
        let written = written(text, n);
        written == name || written.strip_suffix("(..)") == Some(name)
    })?;
    with_commas(node, commas).map(Edit::delete)
}

/// `node`, without the trivia around it, with the commas `commas` says
/// among its siblings as tree-sitter has them: no whitespace.
pub(super) fn with_commas(node: &SyntaxNode, commas: Commas) -> Option<TextRange> {
    let range = code_range(node)?;
    let siblings: Vec<SyntaxElement> = (node.parent()?.children_with_tokens())
        .filter(|e| !matches!(e.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .collect();
    let at = siblings.iter().position(|e| e.as_node() == Some(node))?;
    let is = |i: Option<usize>, kind: SyntaxKind| {
        i.and_then(|i| siblings.get(i))
            .is_some_and(|e| e.kind() == kind)
    };
    let before = is(at.checked_sub(1), SyntaxKind::Comma);
    let after = is(Some(at + 1), SyntaxKind::Comma) && siblings.get(at + 2).is_some();
    let (before, after) = match commas {
        Commas::Nearest => (before && is(Some(at + 1), SyntaxKind::RParen), after),
        Commas::Before => (before, after),
        Commas::BeforeElseAfter => (before, !before && after),
    };
    let start = if before {
        siblings[at - 1].text_range().start()
    } else {
        range.start()
    };
    let end = if after {
        siblings[at + 2].text_range().start()
    } else {
        range.end()
    };
    Some(TextRange::new(start, end))
}
