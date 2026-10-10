//! The optimizer (`Optimize/Module.hs`): a module's canonical AST, with
//! its top-level annotations, made into the graph code generation reads.

use std::collections::BTreeMap;

use nomi_base::Compat;

use crate::canonical::Annotation;
use crate::canonicalize::module::Canonical;
use crate::optimized::LocalGraph;

/// `optimize`: the graph of `module`, whose top-level values have
/// `annotations`. For now, the graph upstream starts from, before any
/// declaration is added (plan 8.1c).
pub fn module(
    _compat: Compat,
    _annotations: &BTreeMap<String, Annotation>,
    _module: &Canonical,
) -> LocalGraph {
    LocalGraph::default()
}
