//! How a type's name shows in a report (`Reporting/Render/Type/
//! Localizer.hs`): unqualified where the module's imports expose it, by
//! the import's alias where they don't, and by its module's full name
//! where nothing imports it.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::Compat;
use nomi_base::diagnostic::doc::{Doc, text};

use crate::canonical::Home;
use crate::hir::{Exposed, Exposing, Import};

/// `Localizer`: each imported module, by name, with its alias and the
/// types it exposes, or `None` for all of them.
#[derive(Clone, Debug)]
pub struct Localizer {
    imports: BTreeMap<String, (Option<String>, Option<BTreeSet<String>>)>,
    /// The release whose name order shows in a record's fields.
    pub compat: Compat,
}

impl Localizer {
    /// `fromModule`: the module itself, exposing everything, then its
    /// imports, the default ones first as the parser adds them, a later
    /// import of a module replacing an earlier.
    pub fn from_module(compat: Compat, name: &str, imports: &[Import]) -> Localizer {
        let mut map = BTreeMap::new();
        map.insert(name.to_owned(), (None, None));
        for import in imports {
            let exposing = match &import.exposing {
                Some(Exposing::Open) => None,
                Some(Exposing::Explicit(items)) => Some(
                    (items.iter())
                        .filter_map(|item| match item {
                            Exposed::Upper(name, _) => Some(name.text.clone()),
                            Exposed::Lower(_) | Exposed::Operator(_) => None,
                        })
                        .collect(),
                ),
                None => Some(BTreeSet::new()),
            };
            let alias = import.alias.as_ref().map(|a| a.text.clone());
            map.insert(import.name.text.clone(), (alias, exposing));
        }
        Localizer {
            imports: map,
            compat,
        }
    }

    /// `fromNames`: each module of `names` exposing everything.
    pub fn from_names<'n>(compat: Compat, names: impl IntoIterator<Item = &'n str>) -> Localizer {
        Localizer {
            compat,
            imports: names
                .into_iter()
                .map(|n| (n.to_owned(), (None, None)))
                .collect(),
        }
    }

    /// `toChars`.
    pub fn to_chars(&self, home: &Home, name: &str) -> String {
        match self.imports.get(&home.module) {
            None => format!("{}.{name}", home.module),
            Some((_, None)) => name.to_owned(),
            Some((alias, Some(exposed))) => {
                if exposed.contains(name) {
                    name.to_owned()
                } else if name == "List" && home.package == "elm/core" && home.module == "List" {
                    "List".to_owned()
                } else {
                    format!("{}.{name}", alias.as_deref().unwrap_or(&home.module))
                }
            }
        }
    }

    /// `toDoc`.
    pub fn to_doc(&self, home: &Home, name: &str) -> Doc {
        text(self.to_chars(home, name))
    }
}
