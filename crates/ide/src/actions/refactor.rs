//! The inferred annotation, as a fix and a refactor, and exposing or
//! unexposing a declaration.

use super::*;

/// `addTypeAnnotationCodeAction`'s edit: the inferred annotation above the
/// definition whose name is at `range`, at its column.
pub(super) fn annotation_edits(context: &Context<'_>, range: TextRange) -> Option<Vec<Edit>> {
    let module = context.module;
    let token = token_at(&module.root, range.start())?;
    let definition = definition_name(&token)?;
    let ty = crate::hover::definition_type(context.index, module, &definition)?;
    let start = token.text_range().start();
    let text = &*module.text;
    let line_start = text[..usize::from(start)].rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..usize::from(start)].chars().count();
    Some(vec![Edit {
        range: TextRange::empty(start),
        text: format!("{} : {ty}\n{}", token.text(), " ".repeat(column)),
    }])
}

pub(super) fn annotation_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (annotation_edits(&fixing.context, range).into_iter())
        .map(|edits| {
            (
                "Add inferred annotation".to_owned(),
                here(fixing, edits),
                None,
            )
        })
        .collect()
}

pub(super) fn annotation_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    if let Some(found) = annotation_edits(&fixing.context, diagnostic.range)
        && !edits.iter().any(|e| e.text == found[0].text)
    {
        edits.extend(found);
    }
}

/// The refactors at `range`, in elm-language-server's registration
/// order: name, action, title and kind.
pub(super) fn refactors(
    fixing: &Fixing<'_>,
    range: TextRange,
    given: &[Diagnosed],
) -> Vec<(&'static str, &'static str, String, ActionKind)> {
    let mut found = Vec::new();
    let root = &fixing.context.module.root;
    let text = &*fixing.context.module.text;
    let Some(token) = token_at(root, range.start()) else {
        return found;
    };
    // An unannotated value's inferred annotation, where no diagnostic
    // offers it already.
    let annotated = definition_name(&token)
        .and_then(|d| d.parent())
        .map(|d| child(&d, SyntaxKind::Signature).is_some());
    if annotated == Some(false) && !given.iter().any(|d| d.code == "missing_type_annotation") {
        found.push((
            "add_type_annotation",
            "add_type_annotation",
            "Add inferred annotation".to_owned(),
            ActionKind::RefactorExtract,
        ));
    }
    let parent = token.parent();
    let parent_kind = parent.as_ref().map(SyntaxNode::kind);
    let named = parent.as_ref().and_then(name_of).as_ref() == Some(&token);
    let in_let = token
        .parent_ancestors()
        .any(|a| a.kind() == SyntaxKind::LetExpr);
    let expose = "expose_unexpose";
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
    if let Some(function) = function
        && !in_let
    {
        if exposes(root, text, &function, true) {
            found.push((
                expose,
                "unexpose_function",
                "Unexpose Function".to_owned(),
                ActionKind::Refactor,
            ));
        } else {
            found.push((
                expose,
                "expose_function",
                "Expose Function".to_owned(),
                ActionKind::Refactor,
            ));
        }
    }
    if token.kind() == SyntaxKind::UpperIdent
        && named
        && matches!(
            parent_kind,
            Some(SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl)
        )
    {
        let alias = if parent_kind == Some(SyntaxKind::TypeAliasDecl) {
            " Alias"
        } else {
            ""
        };
        if exposes(root, text, token.text(), false) {
            found.push((
                expose,
                "unexpose_type",
                format!("Unexpose Type{alias}"),
                ActionKind::Refactor,
            ));
        } else {
            found.push((
                expose,
                "expose_type",
                format!("Expose Type{alias}"),
                ActionKind::Refactor,
            ));
            if parent_kind == Some(SyntaxKind::CustomTypeDecl) {
                found.push((
                    expose,
                    "expose_type_with_variants",
                    "Expose Type with Variants".to_owned(),
                    ActionKind::Refactor,
                ));
            }
        }
    }
    found
}

/// The type of a signature, when `token` is punctuation tree-sitter-elm
/// gives to its outermost `type_expression`: the arrows of a function
/// type written there, and the parentheses around it or its parts.
pub(super) fn signature_type(token: &SyntaxToken) -> Option<SyntaxNode> {
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
    let climb = |mut node: SyntaxNode| {
        while let Some(up) = node
            .parent()
            .filter(|u| u.kind() == SyntaxKind::FunctionType)
        {
            node = up;
        }
        node
    };
    let parent = token.parent()?;
    let top = match parent.kind() {
        SyntaxKind::FunctionType => climb(parent),
        SyntaxKind::ParenType => match parent.parent()? {
            up if up.kind() == SyntaxKind::FunctionType => climb(up),
            _ => parent,
        },
        _ => return None,
    };
    top.parent()
        .is_some_and(|s| s.kind() == SyntaxKind::Signature)
        .then_some(top)
}

/// A refactor's edits, as resolving it gives them.
pub fn refactor_edits(
    index: &Index,
    snapshot: &Snapshot,
    path: &Path,
    action: &str,
    range: TextRange,
) -> Option<Vec<Edit>> {
    let module = index.module(path)?;
    let context = Context {
        snapshot,
        index,
        module,
        path,
    };
    refactor_edits_in(&context, action, range)
}

pub(super) fn refactor_edits_in(
    context: &Context<'_>,
    action: &str,
    range: TextRange,
) -> Option<Vec<Edit>> {
    let module = context.module;
    match action {
        "add_type_annotation" => annotation_edits(context, range),
        "unexpose_function"
        | "unexpose_type"
        | "expose_function"
        | "expose_type"
        | "expose_type_with_variants" => {
            let token = token_at(&module.root, range.start())?;
            let name = match signature_type(&token) {
                Some(ty) => written(&module.text, &ty).to_owned(),
                None => token.text().to_owned(),
            };
            let edit = match action {
                "expose_function" | "expose_type" => {
                    expose_edit(&module.root, &module.text, &name, false)
                }
                "expose_type_with_variants" => expose_edit(&module.root, &module.text, &name, true),
                _ => unexpose_edit(&module.root, &module.text, &name),
            };

            edit.map(|edit| vec![edit])
        }
        _ => None,
    }
}

/// Whether the module exposes `name` (`isExposedFunctionOrPort`,
/// `isExposedTypeOrTypeAlias`): by its exposing list's text.
pub(super) fn exposes(root: &SyntaxNode, text: &str, name: &str, function: bool) -> bool {
    crate::lens::exposes(root, text, name, function)
}

/// The exposed values and types of the module's exposing list
/// (`getModuleExposingListNodes`).
pub(super) fn exposed_nodes(root: &SyntaxNode) -> Vec<SyntaxNode> {
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

pub(super) fn written<'t>(text: &'t str, node: &SyntaxNode) -> &'t str {
    code_range(node).map_or("", |r| &text[std::ops::Range::<usize>::from(r)])
}

/// The edit of the file at `path` that exposes `name`, or unexposes it:
/// elm-language-server's `elm/expose` and `elm/unexpose` requests.
pub fn exposing_edit(snapshot: &Snapshot, path: &Path, name: &str, expose: bool) -> Option<Edit> {
    let text = snapshot.read(path).ok()?;
    let root = nomi_syntax::parse_module(&text).syntax();
    if expose {
        expose_edit(&root, &text, name, false)
    } else {
        unexpose_edit(&root, &text, name)
    }
}

/// `exposeValueInModule`: `, name` after the last exposed, unless the
/// list is `..` or exposes it already.
pub(crate) fn expose_edit(
    root: &SyntaxNode,
    text: &str,
    name: &str,
    with_variants: bool,
) -> Option<Edit> {
    let nodes = exposed_nodes(root);
    let name = if with_variants {
        format!("{name}(..)")
    } else {
        name.to_owned()
    };
    if nodes.iter().any(|n| written(text, n) == name) {
        return None;
    }
    let last = code_range(nodes.last()?)?;
    Some(Edit {
        range: TextRange::empty(last.end()),
        text: format!(", {name}"),
    })
}

/// `unexposedValueInModule`: `name` out of the exposing list, with the
/// comma between it and its neighbor, unless it's the only one.
pub(crate) fn unexpose_edit(root: &SyntaxNode, text: &str, name: &str) -> Option<Edit> {
    let nodes = exposed_nodes(root);
    if nodes.len() <= 1 {
        return None;
    }
    let node = (nodes.iter()).find(|n| {
        let written = written(text, n);
        written == name || written == format!("{name}(..)")
    })?;
    let range = code_range(node)?;
    // Its siblings as tree-sitter has them: no whitespace.
    let siblings: Vec<SyntaxElement> = (node.parent()?.children_with_tokens())
        .filter(|e| !matches!(e.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .collect();
    let at = siblings.iter().position(|e| e.as_node() == Some(node))?;
    let is = |i: Option<usize>, kind: SyntaxKind| {
        i.and_then(|i| siblings.get(i))
            .is_some_and(|e| e.kind() == kind)
    };
    let mut start = range.start();
    let mut end = range.end();
    if is(at.checked_sub(1), SyntaxKind::Comma) && is(Some(at + 1), SyntaxKind::RParen) {
        start = siblings[at - 1].text_range().start();
    }
    if is(Some(at + 1), SyntaxKind::Comma)
        && let Some(after) = siblings.get(at + 2)
    {
        end = after.text_range().start();
    }
    Some(Edit {
        range: TextRange::new(start, end),
        text: String::new(),
    })
}
