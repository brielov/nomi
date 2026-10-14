//! `Generate/Mode.hs`: how code is generated: for development, with the
//! debugger's metadata or without, or optimized, record fields renamed to
//! the shortest names, the most used first.

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_compiler::optimized::GlobalGraph;

use crate::debug::Types;
use crate::name::Name;

/// `Mode`.
#[derive(Clone, Debug)]
pub enum Mode {
    /// Development; with the debugger, and the types it describes
    /// messages by, when `debug` holds them.
    Dev { debug: Option<Types> },
    /// `--optimize`, each record field's short name.
    Prod(BTreeMap<String, Name>),
}

impl Mode {
    /// `isDebug`.
    pub(crate) fn is_debug(&self) -> bool {
        matches!(self, Mode::Dev { debug: Some(_) })
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
        // Before 0.19.3 the graph's order, by bytes, is already the
        // release's.
        if compat == Compat::V0_19_3 {
            fields.sort_by(|a, b| compat.compare_names(a, b));
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
