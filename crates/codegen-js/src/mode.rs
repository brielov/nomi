//! `Generate/Mode.hs`: how code is generated: for development, with the
//! debugger's metadata or without, or optimized, record fields renamed to
//! the shortest names, the most used first.

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_compiler::optimized::GlobalGraph;

use crate::name::Name;

/// `Mode`.
#[derive(Clone, Debug)]
pub enum Mode {
    /// Development; with the debugger when `debug` (its type metadata
    /// comes with plan 8.9).
    Dev { debug: bool },
    /// `--optimize`, each record field's short name.
    Prod(BTreeMap<String, Name>),
}

impl Mode {
    /// `isDebug`.
    pub fn is_debug(&self) -> bool {
        matches!(self, Mode::Dev { debug: true })
    }
}

/// `shortenFieldNames`: the fields of `graph` by how often they are used,
/// the most used given the first short names; fields used as often take
/// them in the release's order of names.
pub fn shorten_field_names(compat: Compat, graph: &GlobalGraph) -> BTreeMap<String, Name> {
    let mut buckets: BTreeMap<i64, Vec<&String>> = BTreeMap::new();
    for (field, frequency) in &graph.fields {
        buckets.entry(*frequency).or_default().push(field);
    }
    let mut short = BTreeMap::new();
    for fields in buckets.values_mut().rev() {
        if compat == Compat::V0_19_3 {
            fields.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        }
        for field in fields.iter() {
            let name = Name::from_int(short.len());
            short.insert((*field).clone(), name);
        }
    }
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_used_fields_get_the_shortest_names() {
        let mut graph = GlobalGraph::default();
        for (field, n) in [("name", 3), ("age", 1), ("id", 3), ("zip", 1)] {
            graph.fields.insert(field.to_owned(), n);
        }
        let short = shorten_field_names(Compat::V0_19_1, &graph);
        let names: Vec<(&str, &str)> = ["id", "name", "age", "zip"]
            .iter()
            .map(|f| (*f, short[*f].as_str()))
            .collect();
        assert_eq!(
            names,
            [("id", "a"), ("name", "b"), ("age", "c"), ("zip", "d")]
        );
        let short = shorten_field_names(Compat::V0_19_3, &graph);
        assert_eq!(short["id"].as_str(), "a");
        assert_eq!(short["age"].as_str(), "c");
    }
}
