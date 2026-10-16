//! A registry for unit tests, its releases told apart by name alone.

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_project::{Constraint, Name, Version};

use crate::index::Known;
use crate::solver::{Packages, Release};

/// The index's releases of each package, and what every release of each
/// needs, which only those listed can be had, each for elm 0.19.
#[derive(Default)]
pub(crate) struct Fixture {
    pub(crate) index: BTreeMap<Name, Known>,
    pub(crate) needs: BTreeMap<Name, Vec<Name>>,
}

impl Packages for Fixture {
    type Error = ();

    fn versions(&self, name: &Name) -> Option<&Known> {
        self.index.get(name)
    }

    fn release(&mut self, name: &Name, _: Version) -> Result<Option<Release>, ()> {
        let anything = Constraint::anything(Compat::LATEST);
        Ok(self.needs.get(name).map(|deps| Release {
            elm: Constraint::until_next_minor(Version::new(0, 19, 0), Compat::LATEST),
            deps: deps.iter().map(|dep| (dep.clone(), anything)).collect(),
        }))
    }
}

/// The package `text` names.
pub(crate) fn name(text: &str) -> Name {
    text.parse().expect("a package name")
}

/// The version `text` names.
pub(crate) fn version(text: &str) -> Version {
    text.parse().expect("a version")
}
