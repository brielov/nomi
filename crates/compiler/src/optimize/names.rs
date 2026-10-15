//! `Optimize/Names.hs`: what optimizing a definition tracks as it goes:
//! the next generated name, the globals the definition needs, and how
//! often it uses each record field. Upstream threads these through its
//! `Tracker` in the order effects run; here they are one value passed by
//! reference, and callers keep that order.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::Compat;
use nomi_base::diagnostic::code::Region;

use crate::canonical::{CtorOpts, Home};
use crate::optimized::{Expr, Global};

#[derive(Debug)]
pub(super) struct Tracker {
    compat: Compat,
    uid: usize,
    pub(super) deps: BTreeSet<Global>,
    pub(super) fields: BTreeMap<String, i64>,
}

impl Tracker {
    /// `run`'s start: nothing generated or needed yet.
    pub(super) fn new(compat: Compat) -> Tracker {
        Tracker {
            compat,
            uid: 0,
            deps: BTreeSet::new(),
            fields: BTreeMap::new(),
        }
    }

    /// `generate`: `_v0`, `_v1` and so on.
    pub(super) fn generate(&mut self) -> String {
        let name = var_index(self.uid);
        self.uid += 1;
        name
    }

    /// `registerKernel`: the kernel module `home` is needed.
    pub(super) fn kernel(&mut self, home: &str) {
        self.deps.insert(Global::kernel(self.compat, home));
    }

    /// `name` of `home` needed.
    fn register(&mut self, home: &Home, name: &str) -> Global {
        let global = Global {
            home: home.clone(),
            name: name.to_owned(),
        };
        self.deps.insert(global.clone());
        global
    }

    /// `registerGlobal`.
    pub(super) fn global(&mut self, home: &Home, name: &str) -> Expr {
        Expr::VarGlobal(self.register(home, name))
    }

    /// `registerCtor`: a constructor as its union is represented; `True`
    /// and `False` are literals.
    pub(super) fn ctor(&mut self, home: &Home, name: &str, index: usize, opts: CtorOpts) -> Expr {
        let global = self.register(home, name);
        let basics = || home.is("elm/core", "Basics");
        match opts {
            CtorOpts::Normal => Expr::VarGlobal(global),
            CtorOpts::Enum if name == "True" && basics() => Expr::Bool(true),
            CtorOpts::Enum if name == "False" && basics() => Expr::Bool(false),
            CtorOpts::Enum => Expr::VarEnum(global, index),
            CtorOpts::Unbox => {
                self.register(&Home::core("Basics"), "identity");
                Expr::VarBox(global)
            }
        }
    }

    /// `registerDebug`: a `Debug` value, used in `home` at `region`.
    pub(super) fn debug(&mut self, name: &str, home: &Home, region: Region) -> Expr {
        self.register(&Home::core("Debug"), name);
        Expr::VarDebug {
            name: name.to_owned(),
            home: home.clone(),
            region,
            unhandled: None,
        }
    }

    /// `registerField`.
    pub(super) fn field(&mut self, name: &str) {
        *self.fields.entry(name.to_owned()).or_default() += 1;
    }
}

/// `Name.fromVarIndex`: `_v` and the index.
pub(super) fn var_index(index: usize) -> String {
    format!("_v{index}")
}
