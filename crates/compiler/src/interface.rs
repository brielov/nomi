//! A module's interface (`Elm/Interface.hs`): what other modules may use
//! of it, and the unit of incrementality (architecture §6). Its encoding
//! is nomi's own, never upstream's `.elmi`, and depends on nothing but
//! the interface, so the engine keys artifacts by its hash.

use std::collections::BTreeMap;

use crate::canonical::{self, Annotation, Associativity};
use crate::encoding::{Decoder, Encoder};

/// `Interface`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// The package the module is in, `author/project`.
    pub home: String,
    pub values: BTreeMap<String, Annotation>,
    pub unions: BTreeMap<String, Union>,
    pub aliases: BTreeMap<String, Alias>,
    pub binops: BTreeMap<String, Binop>,
}

/// Where canonicalization finds the interface of each module a module
/// imports, by name.
pub trait Interfaces: Sync {
    fn get(&self, module: &str) -> Option<&Interface>;
}

impl Interfaces for std::collections::HashMap<String, Interface> {
    fn get(&self, module: &str) -> Option<&Interface> {
        std::collections::HashMap::get(self, module)
    }
}

/// A union as the module exports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Union {
    /// `Type(..)`: the type and its constructors.
    Open(canonical::Union),
    /// `Type`: the type alone.
    Closed(canonical::Union),
    /// Not exported, but needed to read the types of what is.
    Private(canonical::Union),
}

/// An alias as the module exports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Alias {
    Public(canonical::Alias),
    Private(canonical::Alias),
}

/// `Binop`: an operator, the function it stands for, that function's
/// type, and how it groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binop {
    pub function: String,
    pub annotation: Annotation,
    pub associativity: Associativity,
    pub precedence: i64,
}

impl Union {
    /// `toPublicUnion`: the union as an importer sees it.
    pub fn public(&self) -> Option<canonical::Union> {
        match self {
            Union::Open(union) => Some(union.clone()),
            Union::Closed(union) => Some(canonical::Union {
                ctors: Vec::new(),
                count: 0,
                ..union.clone()
            }),
            Union::Private(_) => None,
        }
    }

    /// `extractUnion`.
    pub fn whole(&self) -> &canonical::Union {
        match self {
            Union::Open(union) | Union::Closed(union) | Union::Private(union) => union,
        }
    }
}

impl Alias {
    /// `toPublicAlias`.
    pub fn public(&self) -> Option<&canonical::Alias> {
        match self {
            Alias::Public(alias) => Some(alias),
            Alias::Private(_) => None,
        }
    }

    /// `extractAlias`.
    pub fn whole(&self) -> &canonical::Alias {
        match self {
            Alias::Public(alias) | Alias::Private(alias) => alias,
        }
    }
}

/// A dependency's module as a project sees it (`DependencyInterface`):
/// whole when the package exposes it, and otherwise only the types other
/// interfaces may mention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DependencyInterface {
    Public(Interface),
    Private {
        home: String,
        unions: BTreeMap<String, canonical::Union>,
        aliases: BTreeMap<String, canonical::Alias>,
    },
}

impl DependencyInterface {
    /// `privatize`: a module of a package the project doesn't depend on
    /// directly.
    pub fn privatize(self) -> DependencyInterface {
        match self {
            DependencyInterface::Public(interface) => DependencyInterface::Private {
                home: interface.home,
                unions: (interface.unions.into_iter())
                    .map(|(name, u)| (name, u.whole().clone()))
                    .collect(),
                aliases: (interface.aliases.into_iter())
                    .map(|(name, a)| (name, a.whole().clone()))
                    .collect(),
            },
            private => private,
        }
    }
}

impl Interface {
    /// The interface's bytes: the same for equal interfaces, and different
    /// for different ones.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::default();
        e.interface(self);
        e.finish()
    }

    /// The interface `bytes` encode, if they are one.
    pub fn decode(bytes: &[u8]) -> Option<Interface> {
        let mut d = Decoder::new(bytes);
        let interface = d.interface()?;
        d.end().then_some(interface)
    }
}

impl DependencyInterface {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::default();
        e.dependency(self);
        e.finish()
    }

    pub fn decode(bytes: &[u8]) -> Option<DependencyInterface> {
        let mut d = Decoder::new(bytes);
        let interface = d.dependency()?;
        d.end().then_some(interface)
    }
}

#[cfg(test)]
mod tests;
