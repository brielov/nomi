//! Completion v0 (plan 6.16): after `Alias.`, what that module exports;
//! elsewhere, the module's own declarations, what its imports expose
//! unqualified, and the qualifiers its imports give. Values carry their
//! types. A scope's locals are for later (9.4), as are fields.

use std::collections::BTreeSet;
use std::path::Path;

use nomi_base::TextSize;
use nomi_compiler::canonical::Annotation;
use nomi_compiler::canonicalize::foreign::with_defaults;
use nomi_compiler::hir::{Def, Exposed, Exposing, Import};
use nomi_compiler::interface::{Alias, Interface, Union};
use nomi_compiler::types;
use nomi_compiler::types::localizer::Localizer;
use nomi_compiler::types::render::{self, Context};
use nomi_engine::index::{Index, Indexed};

use crate::hover::localizer;

/// What a completion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Value,
    Type,
    Constructor,
    Module,
}

/// One completion: what to insert, what it is, and its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
}

/// The completions at `offset` in the file at `path`, each label once,
/// in the order of their labels.
pub fn completions(index: &Index, path: &Path, offset: TextSize) -> Vec<Completion> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    let localizer = localizer(index, module);
    let imports = with_defaults(&index.package, &module.hir.imports);
    let mut items = Items::default();
    match qualifier(&module.text, usize::from(offset)) {
        Some(qualifier) => {
            for import in &imports {
                let name = import.alias.as_ref().unwrap_or(&import.name);
                if name.text != qualifier {
                    continue;
                }
                if let Some(interface) = index.interfaces.get(&import.name.text) {
                    items.everything(&localizer, interface);
                }
            }
        }
        None => {
            own(index, module, &localizer, &mut items);
            for import in &imports {
                let Some(interface) = index.interfaces.get(&import.name.text) else {
                    continue;
                };
                exposed(&localizer, import, interface, &mut items);
                let qualifier = import.alias.as_ref().unwrap_or(&import.name).text.clone();
                items.add(qualifier, CompletionKind::Module, None);
            }
        }
    }
    let mut completions = items.0;
    completions.sort_by(|a, b| a.label.cmp(&b.label));
    completions
}

/// The qualifier before the word at `offset`: `Html` in `Html.te|`.
fn qualifier(text: &str, offset: usize) -> Option<String> {
    let before = &text[..offset.min(text.len())];
    let start = before
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .map_or(0, |i| i + 1);
    let word = &before[start..];
    let dot = word.rfind('.')?;
    let qualifier = &word[..dot];
    qualifier
        .starts_with(char::is_uppercase)
        .then(|| qualifier.to_owned())
}

#[derive(Default)]
struct Items(Vec<Completion>, BTreeSet<String>);

impl Items {
    fn add(&mut self, label: String, kind: CompletionKind, detail: Option<String>) {
        if self.1.insert(label.clone()) {
            self.0.push(Completion {
                label,
                kind,
                detail,
            });
        }
    }

    fn value(&mut self, localizer: &Localizer, name: &str, annotation: &Annotation) {
        let ty = render::can_to_doc(localizer, Context::None, &annotation.ty).plain();
        self.add(name.to_owned(), CompletionKind::Value, Some(ty));
    }

    /// The union `name` and, where they are exposed, its constructors.
    fn union(&mut self, name: &str, union: &Union, ctors: bool) {
        let (Union::Open(u) | Union::Closed(u) | Union::Private(u)) = union;
        self.add(name.to_owned(), CompletionKind::Type, None);
        if ctors && matches!(union, Union::Open(_)) {
            for ctor in &u.ctors {
                self.add(
                    ctor.name.clone(),
                    CompletionKind::Constructor,
                    Some(name.to_owned()),
                );
            }
        }
    }

    /// Everything `interface` exports.
    fn everything(&mut self, localizer: &Localizer, interface: &Interface) {
        for (name, annotation) in &interface.values {
            self.value(localizer, name, annotation);
        }
        for (name, union) in &interface.unions {
            if !matches!(union, Union::Private(_)) {
                self.union(name, union, true);
            }
        }
        for (name, alias) in &interface.aliases {
            if matches!(alias, Alias::Public(_)) {
                self.add(name.clone(), CompletionKind::Type, None);
            }
        }
    }
}

/// What `import` exposes unqualified of `interface`.
fn exposed(localizer: &Localizer, import: &Import, interface: &Interface, items: &mut Items) {
    match &import.exposing {
        None => {}
        Some(Exposing::Open) => items.everything(localizer, interface),
        Some(Exposing::Explicit(exposed)) => {
            for item in exposed {
                match item {
                    Exposed::Lower(name) => {
                        if let Some(annotation) = interface.values.get(&name.text) {
                            items.value(localizer, &name.text, annotation);
                        }
                    }
                    Exposed::Upper(name, open) => {
                        if let Some(union) = interface.unions.get(&name.text) {
                            items.union(&name.text, union, open.is_some());
                        } else if interface.aliases.contains_key(&name.text) {
                            items.add(name.text.clone(), CompletionKind::Type, None);
                        }
                    }
                    Exposed::Operator(_) => {}
                }
            }
        }
    }
}

/// The module's own declarations, its values with their types.
fn own(index: &Index, module: &Indexed, localizer: &Localizer, items: &mut Items) {
    let inferred = (module.canonical.as_ref())
        .and_then(|canonical| types::infer(index.compat, canonical).ok())
        .unwrap_or_default();
    for &id in &module.hir.values {
        if let Def::Define { name, .. } = &module.hir.defs[id] {
            match inferred.get(&name.text) {
                Some(annotation) => items.value(localizer, &name.text, annotation),
                None => items.add(name.text.clone(), CompletionKind::Value, None),
            }
        }
    }
    for union in &module.hir.unions {
        items.add(union.name.text.clone(), CompletionKind::Type, None);
        for (ctor, _) in &union.variants {
            items.add(
                ctor.text.clone(),
                CompletionKind::Constructor,
                Some(union.name.text.clone()),
            );
        }
    }
    for alias in &module.hir.aliases {
        items.add(alias.name.text.clone(), CompletionKind::Type, None);
    }
}
