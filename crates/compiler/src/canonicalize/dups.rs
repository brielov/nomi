//! Finding names written twice (`Environment/Dups.hs`). Each name keeps
//! every place it was written, in the order upstream's `OneOrMore` holds
//! them: an insert puts the new place first in 0.19.1 and last from
//! 0.19.2, a record field's insert always puts it first, and a union puts
//! the left side first. A name written twice is reported at its first two
//! places in that order, which is not always the source's.

use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange};

use super::error::{Error, R};

/// `Dups.Dict`.
#[derive(Clone, Debug)]
pub struct Dict<V> {
    names: BTreeMap<String, Vec<(TextRange, V)>>,
}

impl<V> Default for Dict<V> {
    fn default() -> Dict<V> {
        Dict {
            names: BTreeMap::new(),
        }
    }
}

impl<V> Dict<V> {
    /// `Dups.one`.
    pub fn one(name: &str, range: TextRange, value: V) -> Dict<V> {
        let mut dict = Dict::default();
        dict.insert_first(name, range, value);
        dict
    }

    /// `Dups.insert`: the new place goes first in 0.19.1, last from
    /// 0.19.2.
    pub fn insert(&mut self, compat: Compat, name: &str, range: TextRange, value: V) {
        let places = self.names.entry(name.to_owned()).or_default();
        match compat {
            Compat::V0_19_1 => places.insert(0, (range, value)),
            Compat::V0_19_2 | Compat::V0_19_3 => places.push((range, value)),
        }
    }

    /// `addField`'s insert, unchanged in every release: the new place
    /// goes first.
    fn insert_first(&mut self, name: &str, range: TextRange, value: V) {
        self.names
            .entry(name.to_owned())
            .or_default()
            .insert(0, (range, value));
    }

    /// `Dups.union`: `self`'s places before `other`'s.
    pub fn union(mut self, other: Dict<V>) -> Dict<V> {
        for (name, places) in other.names {
            self.names.entry(name).or_default().extend(places);
        }
        self
    }

    /// `Dups.unions`.
    pub fn unions(dicts: impl IntoIterator<Item = Dict<V>>) -> Dict<V> {
        dicts.into_iter().fold(Dict::default(), Dict::union)
    }

    /// `Dups.detect`: each name's value, or an error for every name
    /// written twice, in `compat`'s name order.
    pub fn detect(
        self,
        compat: Compat,
        error: impl Fn(&str, TextRange, TextRange) -> Error,
    ) -> R<BTreeMap<String, V>> {
        let mut values = BTreeMap::new();
        let mut errors = Vec::new();
        let mut names: Vec<(String, Vec<(TextRange, V)>)> = self.names.into_iter().collect();
        names.sort_by(|a, b| compat.compare_names(&a.0, &b.0));
        for (name, mut places) in names {
            if places.len() == 1 {
                values.insert(name, places.pop().expect("one place").1);
            } else {
                errors.push(error(&name, places[0].0, places[1].0));
            }
        }
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }
}

/// `Dups.checkFields`: a record's fields by name, each written once. The
/// fields are added last first, as upstream's `foldr` adds them, so a
/// field written twice is reported in source order.
pub fn check_fields<V>(
    compat: Compat,
    fields: Vec<(String, TextRange, V)>,
) -> R<BTreeMap<String, V>> {
    let mut dict = Dict::default();
    for (name, range, value) in fields.into_iter().rev() {
        dict.insert_first(&name, range, value);
    }
    dict.detect(compat, |name, first, second| Error::DuplicateField {
        name: name.to_owned(),
        first,
        second,
    })
}
