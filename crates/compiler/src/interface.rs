//! A module's interface (`Elm/Interface.hs`): what other modules may use
//! of it, and the unit of incrementality (architecture §6). Its encoding
//! is nomi's own, never upstream's `.elmi`, and depends on nothing but
//! the interface, so the engine keys artifacts by its hash.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::canonical::{self, Annotation};
use crate::encoding::{Decoder, Encoder};
use crate::hir::Fixity;

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
    /// The interface of the module named `module`, if there is one.
    fn get(&self, module: &str) -> Option<&Interface>;
}

impl Interfaces for std::collections::HashMap<String, Interface> {
    fn get(&self, module: &str) -> Option<&Interface> {
        std::collections::HashMap::get(self, module)
    }
}

/// A union as the module exports it, shared with the patterns of its
/// constructors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Union {
    /// `Type(..)`: the type and its constructors.
    Open(Arc<canonical::Union>),
    /// `Type`: the type alone.
    Closed(Arc<canonical::Union>),
    /// Not exported, but needed to read the types of what is.
    Private(Arc<canonical::Union>),
}

/// An alias as the module exports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Alias {
    Public(canonical::Alias),
    /// Not exported, but needed to read the types of what is.
    Private(canonical::Alias),
}

/// `Binop`: an operator, the function it stands for, that function's
/// type, and how it groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binop {
    pub function: String,
    pub annotation: Annotation,
    pub fixity: Fixity,
}

impl Union {
    /// `extractUnion`.
    pub fn whole(&self) -> &canonical::Union {
        match self {
            Union::Open(union) | Union::Closed(union) | Union::Private(union) => union,
        }
    }

    fn into_whole(self) -> Arc<canonical::Union> {
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

    fn into_whole(self) -> canonical::Alias {
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
        unions: BTreeMap<String, Arc<canonical::Union>>,
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
                    .map(|(name, u)| (name, u.into_whole()))
                    .collect(),
                aliases: (interface.aliases.into_iter())
                    .map(|(name, a)| (name, a.into_whole()))
                    .collect(),
            },
            private @ DependencyInterface::Private { .. } => private,
        }
    }

    /// Its bytes, as [`Interface::encode`]'s.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::default();
        e.dependency(self);
        e.finish()
    }

    /// The dependency interface `bytes` encode, if they are one.
    pub fn decode(bytes: &[u8]) -> Option<DependencyInterface> {
        decode_all(bytes, Decoder::dependency)
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
        decode_all(bytes, Decoder::interface)
    }
}

/// What `read` makes of `bytes`, if it reads every one.
fn decode_all<'b, T>(
    bytes: &'b [u8],
    read: impl FnOnce(&mut Decoder<'b>) -> Option<T>,
) -> Option<T> {
    let mut d = Decoder::new(bytes);
    let value = read(&mut d)?;
    d.end().then_some(value)
}

#[cfg(test)]
mod tests;
