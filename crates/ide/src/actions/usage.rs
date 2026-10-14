//! Code a missing name's usage asks for: a local or external function, a
//! new parameter, or a constructor of the type its context expects.

use std::path::PathBuf;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::types::{ExprTypes, error};
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use super::import::MissingName;
use super::refactor::expose_edit;
use super::{Diagnosed, Fixing, Offer, Usage, here, name_of, token_at};
use crate::hover::{Names, render_error};
use crate::problems::Code;
use crate::tree::{child, code_range, declared_type, written_imports};
use crate::{Edit, FileEdits, column, last_line, next_line};

/// What a missing value is made to type its usage.
const TODO: &str = "(Debug.todo \"\")";

/// The types of the module with [`TODO`] in place of each of `ranges`, in
/// order and apart.
pub(super) fn with_todos(fixing: &Fixing<'_>, ranges: &[TextRange]) -> Option<ExprTypes> {
    let edits: Vec<Edit> = (ranges.iter())
        .map(|range| Edit {
            range: *range,
            text: TODO.to_owned(),
        })
        .collect();
    let context = &fixing.context;
    let (_, types) = crate::record_fields::patched_all(context.index, context.module, &edits)?;
    Some(types)
}

/// Where the [`TODO`] in place of `range` is, among those in place of
/// each of `ranges`, in order.
pub(super) fn todo_at(ranges: &[TextRange], range: TextRange) -> TextRange {
    let shift: i64 = (ranges.iter())
        .take_while(|r| r.start() < range.start())
        .map(|r| TODO.len() as i64 - i64::from(u32::from(r.len())))
        .sum();
    let start = TextSize::new((i64::from(u32::from(range.start())) + shift) as u32);
    TextRange::at(start, TextSize::of(TODO))
}

/// The type `types` give the [`TODO`] at `at`, or what's in its
/// parentheses.
pub(super) fn todo_type(types: &ExprTypes, at: TextRange) -> Option<&error::Type> {
    let inner = TextRange::new(at.start() + TextSize::new(1), at.end() - TextSize::new(1));
    (types.iter())
        .rev()
        .find(|(r, _)| *r == at || *r == inner)
        .map(|(_, ty)| ty)
}

/// The type usage gives the missing value at `range`: the module with
/// every missing value made [`TODO`], inferred, and that one's type.
fn usage_type<'f>(fixing: &'f Fixing<'_>, range: TextRange) -> Option<&'f error::Type> {
    let usage = fixing.usage.get_or_init(|| {
        let mut missing: Vec<TextRange> = (fixing.program.iter())
            .filter(|d| d.is(Code::MissingValue))
            .map(|d| d.range)
            .collect();
        missing.sort_by_key(|r| r.start());
        missing.dedup_by(|b, a| b.start() < a.end());
        let types = with_todos(fixing, &missing)?;
        Some(Usage { types, missing })
    });
    let Usage { types, missing } = usage.as_ref()?;
    if !missing.contains(&range) {
        return None;
    }
    todo_type(types, todo_at(missing, range))
}

pub(super) fn render_usage(fixing: &Fixing<'_>, ty: &error::Type) -> String {
    render_error(fixing.context.names(), ty)
}

/// How many parameters a function of type `ty` takes.
fn arity(ty: &error::Type) -> usize {
    match ty {
        error::Type::Lambda(_, _, rest) => 1 + rest.len(),
        _ => 0,
    }
}

/// `createTopLevelFunction`'s text: `name`, typed, taking `arity`
/// arguments, as `Debug.todo`.
fn stub_function(name: &str, ty: &str, arity: usize) -> String {
    let arguments: String = (1..=arity).map(|i| format!(" arg{i}")).collect();
    format!("\n\n{name} : {ty}\n{name}{arguments} =\n    Debug.todo \"TODO\"\n")
}

/// The missing value an unqualified name at `range` is, as a value's name
/// written in an expression.
fn missing_local(fixing: &Fixing<'_>, range: TextRange) -> Option<SyntaxToken> {
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
fn local_function_edit(fixing: &Fixing<'_>, range: TextRange) -> Option<Edit> {
    let token = missing_local(fixing, range)?;
    let text = &*fixing.context.module.text;
    let top = token
        .parent_ancestors()
        .find(|a| a.parent().is_some_and(|p| p.kind() == SyntaxKind::Module));
    let at = match top.and_then(|t| code_range(&t)) {
        Some(top) => next_line(text, top.end()),
        None => last_line(text),
    };
    let ty = usage_type(fixing, range);
    let rendered = ty.map_or_else(|| "a".to_owned(), |ty| render_usage(fixing, ty));
    let function = stub_function(token.text(), &rendered, ty.map_or(0, arity));
    Some(Edit::insert(at, function))
}

pub(super) fn local_function_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (local_function_edit(fixing, range).into_iter())
        .map(|edit| Offer::new("Create local function", here(fixing, vec![edit])))
        .collect()
}

pub(super) fn local_function_all(
    fixing: &Fixing<'_>,
    diagnostic: &Diagnosed,
    edits: &mut Vec<Edit>,
) {
    edits.extend(local_function_edit(fixing, diagnostic.range));
}

/// A function made in another of the project's modules: the module, its
/// file, and the edits there.
struct ExternalFunction {
    module: String,
    path: PathBuf,
    edits: Vec<Edit>,
}

/// `makeExternalDeclarationFromUsageCodeAction`: for a qualified missing
/// value, a function at the end of each project module its qualifier
/// imports, exposed there.
fn external_functions(fixing: &Fixing<'_>, range: TextRange) -> Vec<ExternalFunction> {
    let MissingName { qualifier, name } = MissingName::at(fixing, range);
    let module = fixing.context.module;
    let in_expression = match module.root().covering_element(range) {
        SyntaxElement::Node(node) => node.kind() == SyntaxKind::NameExpr,
        SyntaxElement::Token(token) => token
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NameExpr),
    };
    let Some(qualifier) = qualifier else {
        return Vec::new();
    };
    if !in_expression || !name.starts_with(char::is_lowercase) {
        return Vec::new();
    }
    let ty = usage_type(fixing, range);
    let rendered = ty.map_or_else(|| "a".to_owned(), |ty| render_usage(fixing, ty));
    let arity = ty.map_or(0, arity);
    let mut found = Vec::new();
    for import in written_imports(&module.root()) {
        if import.module != qualifier && import.alias.as_deref() != Some(qualifier) {
            continue;
        }
        // The project's own, which it may write to.
        let Some(target) = fixing.context.index.named(&import.module) else {
            continue;
        };
        let Some(expose) = expose_edit(&target.root(), &target.text, name) else {
            continue;
        };
        let function = stub_function(name, &rendered, arity);
        found.push(ExternalFunction {
            module: import.module,
            path: target.path.clone(),
            edits: vec![Edit::insert(last_line(&target.text), function), expose],
        });
    }
    found
}

pub(super) fn external_function_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (external_functions(fixing, range).into_iter())
        .map(|function| {
            let title = format!("Create function in module '{}'", function.module);
            Offer::new(title, FileEdits::of(&function.path, function.edits))
        })
        .collect()
}

pub(super) fn external_function_all(
    fixing: &Fixing<'_>,
    diagnostic: &Diagnosed,
    files: &mut FileEdits,
) {
    if let Some(function) = external_functions(fixing, diagnostic.range)
        .into_iter()
        .next()
    {
        files.file(&function.path).extend(function.edits);
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
            let written = match ty {
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
            edits.push(Edit::insert(at, format!("{written} -> ")));
        }
        let after = last.text_range().end() + TextSize::new(1);
        edits.push(Edit::insert(after, format!("{} ", token.text())));
        let title = format!("Add new parameter to '{}'", function.text());
        found.push(Offer::new(title, here(fixing, edits)));
    }
    found
}

/// Where a signature's last type starts, as tree-sitter-elm's
/// `type_expression`'s last child: the result type, or the `)` closing it.
fn last_parameter_type(signature: &SyntaxNode) -> Option<TextSize> {
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
fn union_edits(fixing: &Fixing<'_>, range: TextRange) -> Option<FileEdits> {
    let context = &fixing.context;
    let module = context.module;
    let token = token_at(&module.root(), range.start())?;
    let in_expression = token
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::NameExpr);
    if token.kind() != SyntaxKind::UpperIdent || token.text_range() != range || !in_expression {
        return None;
    }
    // A function's arguments, where it gives a custom type.
    let ty = usage_type(fixing, range)?;
    let (params, ty) = match ty {
        error::Type::Lambda(a, b, rest) => {
            let mut all: Vec<&error::Type> = [&**a, &**b].into_iter().chain(rest).collect();
            let result = all.pop().expect("a function's result");
            if matches!(result, error::Type::Type(..)) {
                (all, result)
            } else {
                (Vec::new(), ty)
            }
        }
        ty => (Vec::new(), ty),
    };
    let error::Type::Type(home, name, _) = ty else {
        return None;
    };
    // Its declaration, in this module or an imported one of the project.
    let target = if *home.module == *module.name {
        module
    } else {
        let imported = written_imports(&module.root()).any(|i| i.module == *home.module);
        (imported.then(|| context.index.named(&home.module))).flatten()?
    };
    let declaration = declared_type(&target.root(), SyntaxKind::CustomTypeDecl, name)?;
    let last = declaration
        .children()
        .filter(|c| c.kind() == SyntaxKind::Variant)
        .last()?;
    let last_range = code_range(&last)?;
    // At the column of the `|` or `=` before the last variant.
    let before = (last.first_token()).and_then(|t| {
        std::iter::successors(t.prev_token(), |t| t.prev_token()).find(|t| !t.kind().is_trivia())
    });
    let text = &*target.text;
    let indent = match before {
        Some(t) if matches!(t.kind(), SyntaxKind::Pipe | SyntaxKind::Eq) => {
            column(text, t.text_range().start())
        }
        _ => column(text, last_range.start()),
    };
    let names = Names::of(context.index, target);
    let arguments: String = (params.iter())
        .map(|p| {
            let p = render_error(&names, p);
            if p.contains(' ') {
                format!(" ({p})")
            } else {
                format!(" {p}")
            }
        })
        .collect();
    let constructor = format!("\n{}| {}{arguments}", " ".repeat(indent), token.text());
    let edit = Edit::insert(last_range.end(), constructor);
    Some(FileEdits::of(&target.path, vec![edit]))
}

pub(super) fn union_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (union_edits(fixing, range).into_iter())
        .map(|edits| Offer::new("Create missing union constructor", edits))
        .collect()
}

pub(super) fn union_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, files: &mut FileEdits) {
    if let Some(edits) = union_edits(fixing, diagnostic.range) {
        files.merge(edits);
    }
}
