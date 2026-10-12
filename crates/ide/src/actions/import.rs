//! Fixes that import what's missing: a value or module from the project
//! or its packages, or a package to install.

use super::*;

/// The name a missing-value diagnostic is about: its module as written,
/// empty when unqualified, and its last part.
pub(super) fn missing_name(fixing: &Fixing<'_>, range: TextRange) -> (String, String) {
    let text = &fixing.context.module.text[std::ops::Range::<usize>::from(range)];
    match text.rsplit_once('.') {
        Some((module, name)) => (module.to_owned(), name.to_owned()),
        None => (String::new(), text.to_owned()),
    }
}

/// The imports that would bring the missing name at `range`: one that
/// exposes it, or for a qualified name, a module named so or ending so.
pub(super) fn imports_for(
    fixing: &Fixing<'_>,
    range: TextRange,
) -> Vec<(String, Option<String>, Edit)> {
    let (target, name) = missing_name(fixing, range);
    let qualified = !target.is_empty();
    (importable(&fixing.context).into_iter())
        .filter(|p| {
            p.value == name
                && (!qualified
                    || if target.contains('.') {
                        p.module == target
                    } else {
                        p.module.ends_with(&target)
                    })
        })
        .map(|p| {
            let value =
                (!qualified).then(|| p.value_to_import.clone().unwrap_or_else(|| p.value.clone()));
            let edit = add_import(&fixing.context, &p.module, value.as_deref(), &target);
            (p.module, value, edit)
        })
        .collect()
}

/// `RefactorEditUtils.addImport`.
pub(super) fn add_import(
    context: &Context<'_>,
    module: &str,
    value: Option<&str>,
    alias: &str,
) -> Edit {
    let mut edit = context.add_import(module, module);
    let alias = if !alias.is_empty() && alias != module {
        format!(" as {alias}")
    } else {
        String::new()
    };
    edit.text = match value {
        Some(value) => format!("import {module}{alias} exposing ({value})\n"),
        None => format!("import {module}{alias}\n"),
    };
    edit
}

pub(super) fn import_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let (target, _) = missing_name(fixing, range);
    (imports_for(fixing, range).into_iter())
        .map(|(module, value, edit)| {
            let alias = if !target.is_empty() && module != target {
                format!(" as \"{target}\"")
            } else {
                String::new()
            };
            let title = match value {
                Some(value) => format!("Import '{value}' from module \"{module}\"{alias}"),
                None => format!("Import module \"{module}\"{alias}"),
            };
            (title, here(fixing, vec![edit]), None)
        })
        .collect()
}

pub(super) fn import_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    if let Some((_, _, edit)) = imports_for(fixing, diagnostic.range).into_iter().next()
        && !edits.iter().any(|e| e.text == edit.text)
    {
        edits.push(edit);
    }
}

/// `installPackageCodeAction`: each package that has the missing
/// module, by rank, installed by a command rather than an edit.
pub(super) fn install_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let module = &fixing.context.module.text[std::ops::Range::<usize>::from(range)];
    let mut packages = (fixing.packages.0.get(module).cloned()).unwrap_or_default();
    packages.sort_by(|a, b| compare_ranking(a, b));
    (packages.into_iter())
        .map(|package| {
            let title = format!("Install package \"{package}\"");
            (
                title,
                here(fixing, Vec::new()),
                Some(ActionCommand::InstallPackage { package }),
            )
        })
        .collect()
}
