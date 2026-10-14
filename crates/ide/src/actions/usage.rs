//! Code a missing name's usage asks for: a local or external function, a
//! new parameter, or a constructor of the type its context expects.

use super::*;

/// The type usage gives the missing value at `range`: the module with
/// every missing value made `Debug.todo`, inferred, and that one's type.
pub(super) fn usage_type(fixing: &Fixing<'_>, range: TextRange) -> Option<error::Type> {
    let Usage { types, moved } = fixing
        .usage
        .get_or_init(|| {
            let module = fixing.context.module;
            let mut missing: Vec<TextRange> = (fixing.program.iter())
                .filter(|d| d.code == "missing_value")
                .map(|d| d.range)
                .collect();
            missing.sort_by_key(|r| r.start());
            missing.dedup_by(|b, a| b.start() < a.end());
            let insert = "(Debug.todo \"\")";
            let edits: Vec<(TextRange, String)> =
                missing.iter().map(|r| (*r, insert.to_owned())).collect();
            let (_, types) =
                crate::record_fields::patched_all(fixing.context.index, module, &edits)?;
            // Where each missing value went, by its range before.
            let mut shift: i64 = 0;
            let mut moved = Vec::new();
            for range in missing {
                let start = (i64::from(u32::from(range.start())) + shift) as u32;
                let end = start + insert.len() as u32;
                moved.push((
                    range,
                    TextRange::new(TextSize::new(start), TextSize::new(end)),
                ));
                shift += insert.len() as i64 - i64::from(u32::from(range.len()));
            }
            Some(Usage { types, moved })
        })
        .as_ref()?;
    let (_, at) = moved.iter().find(|(before, _)| *before == range)?;
    let inner = TextRange::new(at.start() + TextSize::new(1), at.end() - TextSize::new(1));
    (types.iter())
        .rev()
        .find(|(r, _)| *r == *at || *r == inner)
        .map(|(_, ty)| ty.clone())
}

pub(super) fn render_usage(fixing: &Fixing<'_>, ty: &error::Type) -> String {
    let names = crate::hover::Names::of(fixing.context.index, fixing.context.module);
    crate::hover::render_error(&names, ty)
}

/// How many parameters a function of type `ty` takes.
pub(super) fn arity(ty: &error::Type) -> usize {
    match ty {
        error::Type::Lambda(_, _, rest) => 1 + rest.len(),
        _ => 0,
    }
}

/// `createTopLevelFunction`'s text: `name`, typed, taking `arity`
/// arguments, as `Debug.todo`.
pub(super) fn new_function(name: &str, ty: &str, arity: usize) -> String {
    let arguments: String = (1..=arity).map(|i| format!(" arg{i}")).collect();
    format!("\n\n{name} : {ty}\n{name}{arguments} =\n    Debug.todo \"TODO\"\n")
}

/// The offset where `line` starts, or the text's end past its last.
pub(super) fn line_start(text: &str, line: usize) -> TextSize {
    let at = if line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 1)
            .map_or(text.len(), |(i, _)| i + 1)
    };
    TextSize::new(at as u32)
}

/// The missing value an unqualified name at `range` is, as a value's name
/// written in an expression.
pub(super) fn missing_local(fixing: &Fixing<'_>, range: TextRange) -> Option<SyntaxToken> {
    let token = token_at(&fixing.context.module.root(), range.start())?;
    (token.kind() == SyntaxKind::LowerIdent
        && token.text_range() == range
        && token
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NameExpr)
        && token
            .prev_token()
            .is_none_or(|t| t.kind() != SyntaxKind::Dot))
    .then_some(token)
}

/// `makeDeclarationFromUsageCodeAction`: a top-level function on the line
/// after the declaration that uses it.
pub(super) fn local_function_edits(fixing: &Fixing<'_>, range: TextRange) -> Option<Edit> {
    let token = missing_local(fixing, range)?;
    let text = &*fixing.context.module.text;
    let top = token
        .parent_ancestors()
        .find(|a| a.parent().is_some_and(|p| p.kind() == SyntaxKind::Module));
    let line = match top.and_then(|t| code_range(&t)) {
        Some(end) => text[..usize::from(end.end())].matches('\n').count() + 1,
        None => text.matches('\n').count(),
    };
    let ty = usage_type(fixing, range);
    let rendered = ty
        .as_ref()
        .map_or_else(|| "a".to_owned(), |ty| render_usage(fixing, ty));
    Some(Edit {
        range: TextRange::empty(line_start(text, line)),
        text: new_function(token.text(), &rendered, ty.as_ref().map_or(0, arity)),
    })
}

pub(super) fn local_function_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (local_function_edits(fixing, range).into_iter())
        .map(|edit| {
            (
                "Create local function".to_owned(),
                here(fixing, vec![edit]),
                None,
            )
        })
        .collect()
}

pub(super) fn local_function_all(
    fixing: &Fixing<'_>,
    diagnostic: &Diagnosed,
    edits: &mut Vec<Edit>,
) {
    edits.extend(local_function_edits(fixing, diagnostic.range));
}

/// `makeExternalDeclarationFromUsageCodeAction`: for a qualified missing
/// value, a function at the end of each project module its qualifier
/// imports, exposed there.
pub(super) fn external_function_edits(
    fixing: &Fixing<'_>,
    range: TextRange,
) -> Vec<(String, PathBuf, Vec<Edit>)> {
    let (qualifier, name) = missing_name(fixing, range);
    let module = fixing.context.module;
    let in_expression = match module.root().covering_element(range) {
        SyntaxElement::Node(node) => node.kind() == SyntaxKind::NameExpr,
        SyntaxElement::Token(token) => token
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NameExpr),
    };
    if qualifier.is_empty() || !in_expression || !name.starts_with(char::is_lowercase) {
        return Vec::new();
    }
    let ty = usage_type(fixing, range);
    let rendered = ty
        .as_ref()
        .map_or_else(|| "a".to_owned(), |ty| render_usage(fixing, ty));
    let arity = ty.as_ref().map_or(0, arity);
    let mut found = Vec::new();
    for import in (module.root().children()).filter(|n| n.kind() == SyntaxKind::Import) {
        let Some(imported) = child(&import, SyntaxKind::ModuleName).map(|n| crate::tree::words(&n))
        else {
            continue;
        };
        let alias = (child(&import, SyntaxKind::ImportAlias))
            .and_then(|a| crate::tree::token(&a, SyntaxKind::UpperIdent));
        if imported != qualifier && alias.as_deref() != Some(qualifier.as_str()) {
            continue;
        }
        // The project's own, which it may write to.
        let Some(target) = fixing.context.index.named(&imported) else {
            continue;
        };
        let Ok(target_text) = fixing.context.snapshot.read(&target.path) else {
            continue;
        };
        let root = nomi_syntax::parse_module(&target_text).syntax();
        let end = line_start(&target_text, target_text.matches('\n').count());
        let Some(expose) = expose_edit(&root, &target_text, &name, false) else {
            continue;
        };
        let function = Edit {
            range: TextRange::empty(end),
            text: new_function(&name, &rendered, arity),
        };
        found.push((imported, target.path.clone(), vec![function, expose]));
    }
    found
}

pub(super) fn external_function_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (external_function_edits(fixing, range).into_iter())
        .map(|(module, path, edits)| {
            (
                format!("Create function in module '{module}'"),
                vec![(path, edits)],
                None,
            )
        })
        .collect()
}

pub(super) fn external_function_all(
    fixing: &Fixing<'_>,
    diagnostic: &Diagnosed,
    files: &mut FileEdits,
) {
    if let Some((_, path, edits)) = external_function_edits(fixing, diagnostic.range)
        .into_iter()
        .next()
    {
        match files.iter_mut().find(|(p, _)| *p == path) {
            Some((_, existing)) => existing.extend(edits),
            None => files.push((path, edits)),
        }
    }
}

/// `addNewFunctionParameter`: the missing value as a new last parameter
/// of each function around it, the innermost first, and in its
/// signature where it has one.
pub(super) fn parameter_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let Some(token) = missing_local(fixing, range) else {
        return Vec::new();
    };
    let ty = usage_type(fixing, range);
    let mut found = Vec::new();
    for declaration in token
        .parent_ancestors()
        .filter(|a| matches!(a.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue))
    {
        let Some(definition) = child(&declaration, SyntaxKind::Definition) else {
            continue;
        };
        let Some(function) = name_of(&definition) else {
            continue;
        };
        // After the last parameter, or the name, one column on.
        let last = (definition.children_with_tokens())
            .filter(|e| !e.kind().is_trivia())
            .take_while(|e| e.kind() != SyntaxKind::Eq)
            .last();
        let Some(last) = last else {
            continue;
        };
        let mut edits = Vec::new();
        if let Some(at) =
            child(&declaration, SyntaxKind::Signature).and_then(|s| last_parameter_type(&s))
        {
            // A type variable is written as the value's name.
            let written = match &ty {
                Some(
                    error::Type::FlexVar(_)
                    | error::Type::FlexSuper(..)
                    | error::Type::RigidVar(_)
                    | error::Type::RigidSuper(..),
                )
                | None => token.text().to_owned(),
                Some(ty) => render_usage(fixing, ty),
            };
            let written = if written.contains(' ') {
                format!("({written})")
            } else {
                written
            };
            edits.push(Edit {
                range: TextRange::empty(at),
                text: format!("{written} -> "),
            });
        }
        edits.push(Edit {
            range: TextRange::empty(last.text_range().end() + TextSize::new(1)),
            text: format!("{} ", token.text()),
        });
        found.push((
            format!("Add new parameter to '{}'", function.text()),
            here(fixing, edits),
            None,
        ));
    }
    found
}

/// Where a signature's last type starts, as tree-sitter-elm's
/// `type_expression`'s last child: the result type, or the `)` closing it.
pub(super) fn last_parameter_type(signature: &SyntaxNode) -> Option<TextSize> {
    let mut ty = signature.children().last()?;
    while ty.kind() == SyntaxKind::FunctionType {
        ty = ty.children().last()?;
    }
    if ty.kind() == SyntaxKind::ParenType {
        return ty.last_token().map(|t| t.text_range().start());
    }
    code_range(&ty).map(|r| r.start())
}

/// `addMissingUnionType`: a missing constructor the context expects of a
/// custom type, added to that type's declaration with the arguments the
/// context gives it, in the project's own modules.
pub(super) fn union_edits(fixing: &Fixing<'_>, range: TextRange) -> FileEdits {
    let context = &fixing.context;
    let module = context.module;
    let Some(token) = token_at(&module.root(), range.start()) else {
        return Vec::new();
    };
    let in_expression = token
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::NameExpr);
    if token.kind() != SyntaxKind::UpperIdent || token.text_range() != range || !in_expression {
        return Vec::new();
    }
    let Some(mut ty) = usage_type(fixing, range) else {
        return Vec::new();
    };
    let mut params = Vec::new();
    if let error::Type::Lambda(a, b, rest) = &ty {
        let mut all: Vec<error::Type> = vec![(**a).clone(), (**b).clone()];
        all.extend(rest.iter().cloned());
        let result = all.pop().expect("a function's result");
        if matches!(result, error::Type::Type(..)) {
            params = all;
            ty = result;
        }
    }
    let error::Type::Type(home, name, _) = &ty else {
        return Vec::new();
    };
    // Its declaration, in this module or an imported one of the project.
    let target = if *home.module == *module.name {
        Some(module)
    } else {
        let imported = (module.root().children())
            .filter(|n| n.kind() == SyntaxKind::Import)
            .filter_map(|i| child(&i, SyntaxKind::ModuleName))
            .any(|n| crate::tree::words(&n) == *home.module);
        imported
            .then(|| context.index.named(&home.module))
            .flatten()
    };
    let Some(target) = target else {
        return Vec::new();
    };
    let Some(declaration) = (target.root().children()).find(|d| {
        d.kind() == SyntaxKind::CustomTypeDecl
            && crate::tree::token(d, SyntaxKind::UpperIdent).as_deref() == Some(name.as_ref())
    }) else {
        return Vec::new();
    };
    let Some(last) = declaration
        .children()
        .filter(|c| c.kind() == SyntaxKind::Variant)
        .last()
    else {
        return Vec::new();
    };
    let Some(end) = code_range(&last).map(|r| r.end()) else {
        return Vec::new();
    };
    // At the column of the `|` or `=` before the last variant.
    let before = (last.first_token()).and_then(|t| {
        std::iter::successors(t.prev_token(), |t| t.prev_token()).find(|t| !t.kind().is_trivia())
    });
    let text = &*target.text;
    let column_of = |at: TextSize| {
        let line_start = text[..usize::from(at)].rfind('\n').map_or(0, |i| i + 1);
        text[line_start..usize::from(at)].chars().count()
    };
    let column = match before {
        Some(t) if matches!(t.kind(), SyntaxKind::Pipe | SyntaxKind::Eq) => {
            column_of(t.text_range().start())
        }
        _ => code_range(&last).map_or(0, |r| column_of(r.start())),
    };
    let names = crate::hover::Names::of(context.index, target);
    let arguments: String = (params.iter())
        .map(|p| {
            let p = crate::hover::render_error(&names, p);
            if p.contains(' ') {
                format!(" ({p})")
            } else {
                format!(" {p}")
            }
        })
        .collect();
    let edit = Edit {
        range: TextRange::empty(end),
        text: format!("\n{}| {}{arguments}", " ".repeat(column), token.text()),
    };
    vec![(target.path.clone(), vec![edit])]
}

pub(super) fn union_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let edits = union_edits(fixing, range);
    if edits.is_empty() {
        return Vec::new();
    }
    vec![("Create missing union constructor".to_owned(), edits, None)]
}

pub(super) fn union_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, files: &mut FileEdits) {
    merge(files, union_edits(fixing, diagnostic.range));
}
