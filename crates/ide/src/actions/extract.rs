//! Extracting a function (`extractFunctionCodeAction.ts`) or a type alias
//! (`extractTypeAliasCodeAction.ts`) from what the range covers. Both are
//! offered where tree-sitter-elm has a node of the right kind exactly at
//! the range, which this module reads off nomi's tree.

use super::*;

/// What tree-sitter-elm's deepest named node exactly at `range` would be
/// called, roughly: enough to tell an expression (`*_expr`) or a type
/// (`*type*`) from the rest. A single name is an identifier there.
fn exact_kind(root: &SyntaxNode, range: TextRange) -> Option<(SyntaxNode, &'static str)> {
    if range.is_empty() {
        return None;
    }
    let mut node = match root.covering_element(range) {
        SyntaxElement::Token(token) => {
            if token.text_range() != range {
                return None;
            }
            // A literal is its own expression node; a name isn't.
            let parent = token.parent()?;
            return (parent.kind() == SyntaxKind::LiteralExpr
                && code_range(&parent) == Some(range))
            .then_some((parent, "literal_expr"));
        }
        SyntaxElement::Node(node) => node,
    };
    if code_range(&node) != Some(range) {
        return None;
    }
    // The deepest node with this range.
    while let Some(inner) = node.children().find(|c| code_range(c) == Some(range)) {
        node = inner;
    }
    let tokens: Vec<SyntaxToken> = (node.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .collect();
    let one_name = tokens.len() == 1
        && matches!(
            tokens[0].kind(),
            SyntaxKind::LowerIdent | SyntaxKind::UpperIdent
        );
    let qualified_name = matches!(node.kind(), SyntaxKind::NameExpr | SyntaxKind::NamedType)
        && tokens.iter().all(|t| {
            matches!(
                t.kind(),
                SyntaxKind::UpperIdent | SyntaxKind::LowerIdent | SyntaxKind::Dot
            )
        });
    if one_name || qualified_name {
        return None;
    }
    let parent = node.parent();
    let parent_kind = parent.as_ref().map(SyntaxNode::kind);
    // tree-sitter-elm's `case` in parentheses runs on to the `)`.
    let closed = || {
        node.last_token()
            .and_then(|t| {
                std::iter::successors(t.next_token(), |t| t.next_token())
                    .find(|t| !t.kind().is_trivia())
            })
            .is_some_and(|t| t.kind() == SyntaxKind::RParen)
    };
    if node.kind() == SyntaxKind::CaseExpr && parent_kind == Some(SyntaxKind::ParenExpr) && closed()
    {
        return None;
    }
    let kind = match node.kind() {
        SyntaxKind::NameExpr => return None,
        SyntaxKind::LiteralExpr => "literal_expr",
        SyntaxKind::NamedType => "type_ref",
        SyntaxKind::RecordType => "record_type",
        SyntaxKind::TupleType => "tuple_type",
        SyntaxKind::FieldType => "field_type",
        // A function type nested in another is part of tree-sitter's flat
        // type_expression, and parentheses are its own tokens.
        SyntaxKind::FunctionType if parent_kind != Some(SyntaxKind::FunctionType) => {
            "type_expression"
        }
        SyntaxKind::ParenType if parent_kind == Some(SyntaxKind::Signature) => "type_expression",
        kind if format!("{kind:?}").ends_with("Expr") || kind == SyntaxKind::OperatorChain => {
            "expr"
        }
        _ => return None,
    };
    Some((node, kind))
}

/// The refactors that extract what `range` covers, in registration order.
pub(super) fn extract_refactors(fixing: &Fixing<'_>, range: TextRange) -> Vec<Refactor> {
    let module = fixing.context.module;
    let mut found = Vec::new();
    if let Some((node, kind)) = exact_kind(&module.root(), range)
        && kind.contains("expr")
        && kind != "type_expression"
    {
        if node.ancestors().any(|a| a.kind() == SyntaxKind::LetExpr) {
            found.push(Refactor::new(
                "extract_function",
                "extract_let",
                "Extract function to enclosing let",
                ActionKind::RefactorExtract,
            ));
        }
        found.push(Refactor::new(
            "extract_function",
            "extract_top_level",
            "Extract function to top level",
            ActionKind::RefactorExtract,
        ));
    }
    if let Some(action) = type_alias_action(&module.root(), &module.text, range) {
        found.push(Refactor::new(
            "extract_type_alias",
            action,
            "Extract type alias",
            ActionKind::RefactorExtract,
        ));
    }
    found
}

/// The top-level part `node` is in: a declaration, or a top-level value's
/// signature or definition, as tree-sitter-elm's file has them.
fn top_level_part(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.ancestors().find(|a| {
        let parent = a.parent();
        match parent.as_ref().map(SyntaxNode::kind) {
            Some(SyntaxKind::Module) => a.kind() != SyntaxKind::ValueDecl,
            Some(SyntaxKind::ValueDecl) => {
                matches!(a.kind(), SyntaxKind::Signature | SyntaxKind::Definition)
                    && parent
                        .and_then(|p| p.parent())
                        .is_some_and(|m| m.kind() == SyntaxKind::Module)
            }
            _ => false,
        }
    })
}

fn line_of(text: &str, at: TextSize) -> usize {
    text[..usize::from(at)].matches('\n').count()
}

fn column_of(text: &str, at: TextSize) -> usize {
    let line_start = text[..usize::from(at)].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..usize::from(at)].chars().count()
}

/// The renaming position after an edit inserted `inserted` at `at`: as
/// many lines lower as it adds when it's above `range`, and one column
/// right inside added parentheses.
fn rename_after(
    text: &str,
    at: TextSize,
    inserted: &str,
    range: TextRange,
    parenthesized: bool,
) -> RenameAt {
    let lines = if line_of(text, at) < line_of(text, range.start()) {
        inserted.matches('\n').count() as u32
    } else {
        0
    };
    RenameAt {
        from: range.start(),
        lines,
        columns: u32::from(parenthesized),
    }
}

/// `createFunction`: `newFunction`, typed or not, taking `args`, with
/// `content` indented four past `target` from where it was at `from`.
fn new_function(
    ty: Option<&str>,
    args: &[String],
    content: &str,
    from: usize,
    target: usize,
) -> String {
    let spaces = " ".repeat(target);
    let body_spaces = " ".repeat(target + 4);
    let difference = (target + 4) as isize - from as isize;
    let content = if difference > 0 {
        content.replace('\n', &format!("\n{}", " ".repeat(difference as usize)))
    } else if difference < 0 {
        (content.split('\n'))
            .map(|line| {
                let strip = line.len() - line.trim_start_matches(' ').len();
                &line[strip.min(difference.unsigned_abs())..]
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content.to_owned()
    };
    let annotation = ty.map_or(String::new(), |ty| format!("{spaces}newFunction : {ty}\n"));
    let arguments: String = args.iter().map(|a| format!(" {a}")).collect();
    format!("\n\n{annotation}{spaces}newFunction{arguments} =\n{body_spaces}{content}\n")
}

/// `extractFunctionCodeAction`'s edits: the function, and its call in
/// place of what it was made from.
pub(super) fn function_edits(
    context: &Context<'_>,
    action: &str,
    range: TextRange,
) -> Option<(Vec<Edit>, Option<RenameAt>)> {
    let module = context.module;
    let (root, text) = (&module.root(), &*module.text);
    let (mut node, _) = exact_kind(root, range)?;
    let mut parenthesized = node.kind() == SyntaxKind::ParenExpr;
    if parenthesized {
        node = node.children().next()?;
    }
    if matches!(
        node.kind(),
        SyntaxKind::ListExpr
            | SyntaxKind::RecordExpr
            | SyntaxKind::RecordUpdateExpr
            | SyntaxKind::TupleExpr
    ) {
        parenthesized = true;
    }
    let node_range = code_range(&node)?;
    let (insert_at, target_column, target_let, annotate) = if action == "extract_let" {
        let let_expr = node.ancestors().find(|a| a.kind() == SyntaxKind::LetExpr)?;
        let last = (let_expr.children())
            .filter(|c| matches!(c.kind(), SyntaxKind::LetValue | SyntaxKind::LetDestructure))
            .last()?;
        let line = line_of(text, code_range(&last)?.end()) + 1;
        let column = column_of(text, code_range(&let_expr)?.start()) + 4;
        (line_start(text, line), column, Some(let_expr), false)
    } else {
        let line = match node
            .ancestors()
            .find(|a| a.parent().is_some_and(|p| p.kind() == SyntaxKind::Module))
        {
            Some(top) => line_of(text, code_range(&top)?.end()) + 1,
            None => text.matches('\n').count(),
        };
        (line_start(text, line), 0, None, true)
    };
    let args = arguments(context, &node, target_let.as_ref());
    let ty = annotate
        .then(|| function_type(context, &node, &args))
        .flatten();
    let names: Vec<String> = args.iter().map(|a| a.text().to_owned()).collect();
    let content = &text[std::ops::Range::<usize>::from(node_range)];
    let function = new_function(
        ty.as_deref(),
        &names,
        content,
        column_of(text, node_range.start()),
        target_column,
    );
    let call = if names.is_empty() {
        "newFunction".to_owned()
    } else {
        format!("newFunction {}", names.join(" "))
    };
    let parenthesized = parenthesized && !names.is_empty();
    let call = if parenthesized {
        format!("({call})")
    } else {
        call
    };
    let rename = rename_after(text, insert_at, &function, range, parenthesized);
    let edits = vec![
        Edit {
            range: TextRange::empty(insert_at),
            text: function,
        },
        Edit { range, text: call },
    ];
    Some((edits, Some(rename)))
}

/// The names the extracted `node` uses that the place it goes can't see:
/// bound outside it but not where it goes, or nowhere at all.
fn arguments(
    context: &Context<'_>,
    node: &SyntaxNode,
    target_let: Option<&SyntaxNode>,
) -> Vec<SyntaxToken> {
    let module = context.module;
    let root = &module.root();
    let within = node.text_range();
    let mut found: Vec<SyntaxToken> = Vec::new();
    for reference in node.descendants().filter(|n| {
        matches!(
            n.kind(),
            SyntaxKind::NameExpr | SyntaxKind::RecordUpdateExpr
        )
    }) {
        let tokens: Vec<SyntaxToken> = (reference.children_with_tokens())
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia())
            .collect();
        let name = match reference.kind() {
            // Unqualified values only: types and qualified ones are seen.
            SyntaxKind::NameExpr
                if tokens.len() == 1 && tokens[0].kind() == SyntaxKind::LowerIdent =>
            {
                tokens[0].clone()
            }
            SyntaxKind::RecordUpdateExpr => {
                match tokens.iter().find(|t| t.kind() == SyntaxKind::LowerIdent) {
                    Some(base) => base.clone(),
                    None => continue,
                }
            }
            _ => continue,
        };
        if found.iter().any(|a| a.text() == name.text()) {
            continue;
        }
        let at = name.text_range().start();
        match crate::scopes::local(root, at) {
            Some((binder, scope)) => {
                if within.contains_range(binder) {
                    continue;
                }
                // A `let` sees what's in scope around it.
                let seen =
                    target_let.is_some_and(|l| scope.text_range().contains_range(l.text_range()));
                if seen {
                    continue;
                }
            }
            None => {
                // Top-level values and imports are seen from anywhere.
                if crate::navigation::definition(context.snapshot, context.index, context.path, at)
                    .is_some()
                {
                    continue;
                }
            }
        }
        found.push(name);
    }
    found
}

/// The extracted function's type: its arguments' and its body's, as the
/// module infers them.
fn function_type(context: &Context<'_>, node: &SyntaxNode, args: &[SyntaxToken]) -> Option<String> {
    let module = context.module;
    let (_, types) =
        nomi_compiler::types::infer_types(context.index.compat, module.canonical.as_ref()?);
    let type_at = |range: TextRange| {
        (types.iter())
            .rev()
            .find(|(r, _)| crate::hover::trimmed(&module.root(), *r) == Some(range))
            .map(|(_, ty)| ty.clone())
    };
    let body = type_at(code_range(node)?)?;
    let ty = if args.is_empty() {
        body
    } else {
        let mut parts: Vec<error::Type> = (args.iter())
            .map(|a| a.parent().and_then(|p| code_range(&p)).and_then(type_at))
            .collect::<Option<_>>()?;
        parts.push(body);
        let first = parts.remove(0);
        let second = parts.remove(0);
        error::Type::Lambda(Box::new(first), Box::new(second), parts)
    };
    let names = crate::hover::Names::of(context.index, module);
    Some(crate::hover::render_error(&names, &ty))
}

/// The tree-sitter `type_expression` `node` is in: the outermost of a chain
/// of function types, or the type a signature or parentheses hold.
fn type_expression_of(node: &SyntaxNode) -> Option<SyntaxNode> {
    let mut current = node.clone();
    loop {
        let parent = current.parent()?;
        match parent.kind() {
            SyntaxKind::FunctionType => {
                let mut top = parent;
                while let Some(up) = top
                    .parent()
                    .filter(|u| u.kind() == SyntaxKind::FunctionType)
                {
                    top = up;
                }
                return Some(top);
            }
            SyntaxKind::Signature | SyntaxKind::ParenType => return Some(current),
            _ => current = parent,
        }
    }
}

/// Which type alias extraction applies at `range`: of a type exactly
/// there, of one in parentheses, or of a run of a function's parameters.
fn type_alias_action(root: &SyntaxNode, text: &str, range: TextRange) -> Option<&'static str> {
    if exact_kind(root, range).is_some_and(|(_, kind)| kind.contains("type")) {
        return Some("extract_type_alias");
    }
    let start = token_at(root, range.start())?;
    let before_end = text[..usize::from(range.end())].chars().last();
    if before_end == Some(')') {
        let close = root
            .token_at_offset(range.end() - TextSize::new(1))
            .right_biased()?;
        if start.kind() == SyntaxKind::LParen
            && close.kind() == SyntaxKind::RParen
            && start
                .parent()
                .is_some_and(|p| p.kind() == SyntaxKind::ParenType)
            && start.parent() == close.parent()
        {
            return Some("extract_type_alias_parenthesized_expr");
        }
    }
    let end = token_at(root, range.end())?;
    let named = |t: &SyntaxToken| {
        t.parent_ancestors()
            .find(|a| a.kind() == SyntaxKind::NamedType)
    };
    let (first, last) = (named(&start)?, named(&end)?);
    let (first_range, last_range) = (code_range(&first)?, code_range(&last)?);
    (type_expression_of(&first)? == type_expression_of(&last)?
        && first_range.start() == range.start()
        && last_range.end() == range.end())
    .then_some("extract_type_alias_partial_type_expr")
}

/// `extractTypeAliasCodeAction`'s edits: `NewType`, declared above the
/// declaration, in place of the types it was made from.
pub(super) fn type_alias_edits(
    context: &Context<'_>,
    action: &str,
    range: TextRange,
) -> Option<(Vec<Edit>, Option<RenameAt>)> {
    let module = context.module;
    let (root, text) = (&module.root(), &*module.text);
    let nodes: Vec<SyntaxNode> = match action {
        "extract_type_alias_partial_type_expr" => {
            let start = token_at(root, range.start())?;
            let end = token_at(root, range.end())?;
            let first = start
                .parent_ancestors()
                .find(|a| a.kind() == SyntaxKind::NamedType)?;
            let expression = type_expression_of(&first)?;
            // Its parts as tree-sitter's flat type_expression has them.
            let mut parts = Vec::new();
            let mut current = Some(expression);
            while let Some(ty) = current.take() {
                if ty.kind() == SyntaxKind::FunctionType {
                    let mut children = ty.children();
                    parts.extend(children.next());
                    current = children.next();
                } else {
                    parts.push(ty);
                }
            }
            (parts.into_iter())
                .filter(|n| {
                    n.kind() == SyntaxKind::NamedType
                        && n.text_range().start() >= start.text_range().start()
                        && n.text_range().end() <= end.text_range().end()
                })
                .collect()
        }
        "extract_type_alias_parenthesized_expr" => {
            let open = token_at(root, range.start())?;
            vec![open.parent()?.children().next()?]
        }
        _ => vec![exact_kind(root, range)?.0],
    };
    let first = nodes.first()?;
    let line = match top_level_part(first) {
        Some(top) => line_of(text, code_range(&top)?.start()).saturating_sub(1),
        None => text.matches('\n').count(),
    };
    let insert_at = line_start(text, line);
    let mut args: Vec<String> = Vec::new();
    for node in &nodes {
        for var in node
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::TypeVar)
        {
            let name = crate::symbols::words(&var);
            if !args.contains(&name) {
                args.push(name);
            }
        }
    }
    let written: Vec<&str> = (nodes.iter())
        .filter_map(|n| code_range(n).map(|r| &text[std::ops::Range::<usize>::from(r)]))
        .collect();
    let parameters: String = args.iter().map(|a| format!(" {a}")).collect();
    let alias = format!(
        "\ntype alias NewType{parameters} = {}\n\n",
        written.join(" -> ")
    );
    let parenthesized = action == "extract_type_alias_parenthesized_expr" && !args.is_empty();
    let replacement = format!("NewType{parameters}");
    let replacement = if parenthesized {
        format!("({replacement})")
    } else {
        replacement
    };
    let rename = rename_after(text, insert_at, &alias, range, parenthesized);
    let edits = vec![
        Edit {
            range: TextRange::empty(insert_at),
            text: alias,
        },
        Edit {
            range,
            text: replacement,
        },
    ];
    Some((edits, Some(rename)))
}
