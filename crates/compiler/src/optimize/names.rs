//! `Optimize/Names.hs`: what optimizing a definition tracks as it goes:
//! the next generated name, the globals the definition needs, and how
//! often it uses each record field. Upstream threads these through its
//! `Tracker` in the order effects run; here they are one value passed by
//! reference, and callers keep that order.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::Compat;

use crate::canonical::{CtorOpts, Home};
use crate::optimized::{Expr, Global, Region};

#[derive(Debug)]
pub struct Tracker {
    compat: Compat,
    uid: usize,
    pub deps: BTreeSet<Global>,
    pub fields: BTreeMap<String, i64>,
}

impl Tracker {
    /// `run`'s start: nothing generated or needed yet.
    pub fn new(compat: Compat) -> Tracker {
        Tracker {
            compat,
            uid: 0,
            deps: BTreeSet::new(),
            fields: BTreeMap::new(),
        }
    }

    /// `generate`: `_v0`, `_v1` and so on.
    pub fn generate(&mut self) -> String {
        let name = format!("_v{}", self.uid);
        self.uid += 1;
        name
    }

    /// `registerKernel`: the kernel module `home` is needed.
    pub fn kernel(&mut self, home: &str) {
        self.deps.insert(Global::kernel(self.compat, home));
    }

    /// `registerGlobal`.
    pub fn global(&mut self, home: &Home, name: &str) -> Expr {
        let global = Global {
            home: home.clone(),
            name: name.to_owned(),
        };
        self.deps.insert(global.clone());
        Expr::VarGlobal(global)
    }

    /// `registerCtor`: a constructor as its union is represented; `True`
    /// and `False` are literals.
    pub fn ctor(&mut self, home: &Home, name: &str, index: usize, opts: CtorOpts) -> Expr {
        let global = Global {
            home: home.clone(),
            name: name.to_owned(),
        };
        self.deps.insert(global.clone());
        match opts {
            CtorOpts::Normal => Expr::VarGlobal(global),
            CtorOpts::Enum if *home == basics() && name == "True" => Expr::Bool(true),
            CtorOpts::Enum if *home == basics() && name == "False" => Expr::Bool(false),
            CtorOpts::Enum => Expr::VarEnum(global, index),
            CtorOpts::Unbox => {
                self.deps.insert(Global {
                    home: basics(),
                    name: "identity".to_owned(),
                });
                Expr::VarBox(global)
            }
        }
    }

    /// `registerField`.
    pub fn field(&mut self, name: &str) {
        *self.fields.entry(name.to_owned()).or_default() += 1;
    }
}

/// `ModuleName.basics`.
pub fn basics() -> Home {
    Home {
        package: "elm/core".into(),
        module: "Basics".into(),
    }
}

/// `ModuleName.debug`.
pub fn debug() -> Home {
    Home {
        package: "elm/core".into(),
        module: "Debug".into(),
    }
}

/// `registerDebug`: a `Debug` value, used in `home` at `region`.
pub fn debug_value(tracker: &mut Tracker, name: &str, home: &Home, region: Region) -> Expr {
    tracker.deps.insert(Global {
        home: debug(),
        name: name.to_owned(),
    });
    Expr::VarDebug(name.to_owned(), home.clone(), region, None)
}
