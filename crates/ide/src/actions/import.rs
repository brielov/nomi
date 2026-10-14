//! Fixes that import what's missing: a value or module from the project
//! or its packages, or a package to install.

use nomi_base::TextRange;

use super::{ActionCommand, Diagnosed, Fixing, Offer, here};
use crate::Edit;
use crate::completion_general::compare_ranking;

/// The name a missing-value diagnostic is about: its qualifier as
/// written, where it has one, and its last part.
pub(super) struct MissingName<'t> {
    pub(super) qualifier: Option<&'t str>,
    pub(super) name: &'t str,
}

impl<'t> MissingName<'t> {
    pub(super) fn at(fixing: &Fixing<'t>, range: TextRange) -> MissingName<'t> {
        let module = fixing.context.module;
        let text = &module.text[range];
        let (qualifier, name) = match text.rsplit_once('.') {
            Some((qualifier, name)) => (Some(qualifier).filter(|q| !q.is_empty()), name),
            None => (None, text),
        };
        MissingName { qualifier, name }
    }
}

/// An import that brings a missing name: of `module`, exposing `value`
/// where the name is unqualified.
struct ImportFix<'p> {
    module: &'p str,
    value: Option<&'p str>,
    edit: Edit,
}

/// The imports that would bring `missing`: one that exposes it, or for a
/// qualified name, a module named so or ending so.
fn imports_for<'f>(
    fixing: &'f Fixing<'_>,
    missing: &'f MissingName<'_>,
) -> impl Iterator<Item = ImportFix<'f>> {
    let fits = |module: &str| match missing.qualifier {
        None => true,
        Some(qualifier) if qualifier.contains('.') => module == qualifier,
        Some(qualifier) => module.ends_with(qualifier),
    };
    (fixing.context.possible_imports().iter())
        .filter(move |p| p.value == missing.name && fits(&p.module))
        .map(move |p| {
            let value = missing.qualifier.is_none().then(|| {
                let value = p.value_to_import.as_deref();
                value.unwrap_or(&p.value)
            });
            let edit = (fixing.context).add_import(&p.module, missing.qualifier, value);
            ImportFix {
                module: &p.module,
                value,
                edit,
            }
        })
}

pub(super) fn import_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let missing = MissingName::at(fixing, range);
    (imports_for(fixing, &missing))
        .map(|fix| {
            let alias = match missing.qualifier {
                Some(qualifier) if fix.module != qualifier => format!(" as \"{qualifier}\""),
                _ => String::new(),
            };
            let module = fix.module;
            let title = match fix.value {
                Some(value) => format!("Import '{value}' from module \"{module}\"{alias}"),
                None => format!("Import module \"{module}\"{alias}"),
            };
            Offer::new(title, here(fixing, vec![fix.edit]))
        })
        .collect()
}

pub(super) fn import_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    let missing = MissingName::at(fixing, diagnostic.range);
    if let Some(fix) = imports_for(fixing, &missing).next()
        && !edits.iter().any(|e| e.text == fix.edit.text)
    {
        edits.push(fix.edit);
    }
}

/// `installPackageCodeAction`: each package that has the missing
/// module, by rank, installed by a command rather than an edit.
pub(super) fn install_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let module = &fixing.context.module.text[range];
    let mut packages = fixing.request.packages.exposing(module).to_vec();
    packages.sort_by(|a, b| compare_ranking(a, b));
    (packages.into_iter())
        .map(|package| Offer {
            title: format!("Install package \"{package}\""),
            edits: here(fixing, Vec::new()),
            command: Some(ActionCommand::InstallPackage { package }),
        })
        .collect()
}
