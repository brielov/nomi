//! What each name in a module refers to, a side table canonicalization
//! fills as it resolves names (architecture §9), for navigation. It is
//! kept as far as canonicalization got, so a module with naming errors
//! still has the references before them.

use std::cell::RefCell;
use std::rc::Rc;

use nomi_base::TextRange;

use crate::canonical::Home;

/// What a top-level name is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Value,
    Type,
    /// A custom type's variant, or a record alias as its constructor.
    Ctor,
    Binop,
    Module,
}

/// What a name refers to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A local variable, by the range of the name that binds it.
    Local(TextRange),
    /// A top-level name of the module at `home`, this one included; a
    /// module by its own name.
    Global {
        home: Home,
        name: String,
        kind: Kind,
    },
}

/// A name, by its range, and what it refers to. A range is the node's,
/// leading trivia and a qualifier included.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Reference {
    pub range: TextRange,
    pub target: Target,
}

/// Where canonicalization records references: shared, so every scope's
/// copy of the environment records into one table.
#[derive(Clone, Debug, Default)]
pub(crate) struct Recorder(Rc<RefCell<Vec<Reference>>>);

impl Recorder {
    /// Records the name at `range`; none where it is empty, as for the
    /// default imports, which nothing writes.
    pub(crate) fn record(&self, range: TextRange, target: Target) {
        if !range.is_empty() {
            self.0.borrow_mut().push(Reference { range, target });
        }
    }

    /// Records the name at `range` as the top-level `name` of `home`.
    pub(crate) fn global(&self, range: TextRange, home: &Home, name: &str, kind: Kind) {
        let target = Target::Global {
            home: home.clone(),
            name: name.to_owned(),
            kind,
        };
        self.record(range, target);
    }

    /// The references, in source order, each once: an alias's body is
    /// canonicalized twice, by `addTypes` and `canonicalizeAlias` alike.
    pub(crate) fn take(&self) -> Vec<Reference> {
        let mut references = std::mem::take(&mut *self.0.borrow_mut());
        references.sort_by_key(|r| (r.range.start(), r.range.end()));
        let mut seen = std::collections::HashSet::new();
        references.retain(|r| seen.insert(r.clone()));
        references
    }
}
