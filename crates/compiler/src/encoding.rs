//! The bytes of interfaces: tags as one byte, lengths and numbers as
//! little-endian fixed widths, strings as their length and UTF-8, maps in
//! key order. Decoding takes nothing on trust: any malformed input is
//! `None`.

use std::collections::BTreeMap;

use crate::canonical::{
    Alias, AliasType, Annotation, Associativity, Ctor, CtorOpts, FieldType, Home, Type, Union,
};
use crate::interface::{self, Binop, DependencyInterface, Interface};

/// Bumped whenever the encoding changes, so old bytes never decode as new.
const VERSION: u8 = 1;

#[derive(Default)]
pub struct Encoder {
    out: Vec<u8>,
}

impl Encoder {
    pub fn finish(self) -> Vec<u8> {
        self.out
    }

    fn u8(&mut self, n: u8) {
        self.out.push(n);
    }

    fn u64(&mut self, n: u64) {
        self.out.extend_from_slice(&n.to_le_bytes());
    }

    fn len(&mut self, n: usize) {
        self.u64(n as u64);
    }

    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.out.extend_from_slice(s.as_bytes());
    }

    fn strs(&mut self, items: &[String]) {
        self.len(items.len());
        for s in items {
            self.str(s);
        }
    }

    fn map<V>(&mut self, map: &BTreeMap<String, V>, mut value: impl FnMut(&mut Encoder, &V)) {
        self.len(map.len());
        for (key, v) in map {
            self.str(key);
            value(self, v);
        }
    }

    pub fn interface(&mut self, i: &Interface) {
        self.u8(VERSION);
        self.interface_body(i);
    }

    fn interface_body(&mut self, i: &Interface) {
        self.str(&i.home);
        self.map(&i.values, Encoder::annotation);
        self.map(&i.unions, |e, u| {
            let (tag, union) = match u {
                interface::Union::Open(u) => (0, u),
                interface::Union::Closed(u) => (1, u),
                interface::Union::Private(u) => (2, u),
            };
            e.u8(tag);
            e.union(union);
        });
        self.map(&i.aliases, |e, a| {
            let (tag, alias) = match a {
                interface::Alias::Public(a) => (0, a),
                interface::Alias::Private(a) => (1, a),
            };
            e.u8(tag);
            e.alias(alias);
        });
        self.map(&i.binops, |e, b| {
            e.str(&b.function);
            e.annotation(&b.annotation);
            e.u8(match b.associativity {
                Associativity::Left => 0,
                Associativity::Non => 1,
                Associativity::Right => 2,
            });
            e.u64(b.precedence as u64);
        });
    }

    pub fn dependency(&mut self, d: &DependencyInterface) {
        self.u8(VERSION);
        match d {
            DependencyInterface::Public(i) => {
                self.u8(0);
                self.interface_body(i);
            }
            DependencyInterface::Private {
                home,
                unions,
                aliases,
            } => {
                self.u8(1);
                self.str(home);
                self.map(unions, Encoder::union);
                self.map(aliases, Encoder::alias);
            }
        }
    }

    fn annotation(&mut self, a: &Annotation) {
        self.strs(&a.free);
        self.ty(&a.ty);
    }

    fn alias(&mut self, a: &Alias) {
        self.strs(&a.vars);
        self.ty(&a.ty);
    }

    fn union(&mut self, u: &Union) {
        self.strs(&u.vars);
        self.len(u.ctors.len());
        for ctor in &u.ctors {
            self.str(&ctor.name);
            self.len(ctor.index);
            self.len(ctor.args.len());
            for arg in &ctor.args {
                self.ty(arg);
            }
        }
        self.len(u.count);
        self.u8(match u.opts {
            CtorOpts::Normal => 0,
            CtorOpts::Enum => 1,
            CtorOpts::Unbox => 2,
        });
    }

    fn home(&mut self, home: &Home) {
        self.str(&home.package);
        self.str(&home.module);
    }

    fn ty(&mut self, t: &Type) {
        stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || self.ty_(t));
    }

    fn ty_(&mut self, t: &Type) {
        match t {
            Type::Lambda(a, b) => {
                self.u8(0);
                self.ty(a);
                self.ty(b);
            }
            Type::Var(name) => {
                self.u8(1);
                self.str(name);
            }
            Type::Type(home, name, args) => {
                self.u8(2);
                self.home(home);
                self.str(name);
                self.len(args.len());
                for arg in args {
                    self.ty(arg);
                }
            }
            Type::Record(fields, ext) => {
                self.u8(3);
                self.map(fields, |e, f| {
                    e.u64(u64::from(f.order));
                    e.ty(&f.ty);
                });
                match ext {
                    None => self.u8(0),
                    Some(name) => {
                        self.u8(1);
                        self.str(name);
                    }
                }
            }
            Type::Unit => self.u8(4),
            Type::Tuple(a, b, c) => {
                self.u8(5);
                self.ty(a);
                self.ty(b);
                match c {
                    None => self.u8(0),
                    Some(c) => {
                        self.u8(1);
                        self.ty(c);
                    }
                }
            }
            Type::Alias(home, name, args, aliased) => {
                self.u8(6);
                self.home(home);
                self.str(name);
                self.len(args.len());
                for (var, arg) in args {
                    self.str(var);
                    self.ty(arg);
                }
                let (tag, body) = match aliased {
                    AliasType::Holey(t) => (0, t),
                    AliasType::Filled(t) => (1, t),
                };
                self.u8(tag);
                self.ty(body);
            }
        }
    }
}

pub struct Decoder<'a> {
    rest: &'a [u8],
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Decoder<'a> {
        Decoder { rest: bytes }
    }

    /// Whether every byte was read.
    pub fn end(&self) -> bool {
        self.rest.is_empty()
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.rest.len() {
            return None;
        }
        let (taken, rest) = self.rest.split_at(n);
        self.rest = rest;
        Some(taken)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    /// A length, which can't exceed what is left to read.
    fn len(&mut self) -> Option<usize> {
        let n = usize::try_from(self.u64()?).ok()?;
        (n <= self.rest.len()).then_some(n)
    }

    fn str(&mut self) -> Option<String> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn strs(&mut self) -> Option<Vec<String>> {
        let n = self.len()?;
        (0..n).map(|_| self.str()).collect()
    }

    fn map<V>(
        &mut self,
        mut value: impl FnMut(&mut Self) -> Option<V>,
    ) -> Option<BTreeMap<String, V>> {
        let n = self.len()?;
        let mut map = BTreeMap::new();
        let mut last: Option<String> = None;
        for _ in 0..n {
            let key = self.str()?;
            // Keys come in order, once each, as encoding writes them.
            if last.as_ref().is_some_and(|last| *last >= key) {
                return None;
            }
            last = Some(key.clone());
            map.insert(key, value(self)?);
        }
        Some(map)
    }

    pub fn interface(&mut self) -> Option<Interface> {
        (self.u8()? == VERSION).then_some(())?;
        self.interface_body()
    }

    fn interface_body(&mut self) -> Option<Interface> {
        let home = self.str()?;
        let values = self.map(Decoder::annotation)?;
        let unions = self.map(|d| {
            let tag = d.u8()?;
            let union = d.union()?;
            match tag {
                0 => Some(interface::Union::Open(union)),
                1 => Some(interface::Union::Closed(union)),
                2 => Some(interface::Union::Private(union)),
                _ => None,
            }
        })?;
        let aliases = self.map(|d| {
            let tag = d.u8()?;
            let alias = d.alias()?;
            match tag {
                0 => Some(interface::Alias::Public(alias)),
                1 => Some(interface::Alias::Private(alias)),
                _ => None,
            }
        })?;
        let binops = self.map(|d| {
            let function = d.str()?;
            let annotation = d.annotation()?;
            let associativity = match d.u8()? {
                0 => Associativity::Left,
                1 => Associativity::Non,
                2 => Associativity::Right,
                _ => return None,
            };
            let precedence = d.u64()? as i64;
            Some(Binop {
                function,
                annotation,
                associativity,
                precedence,
            })
        })?;
        Some(Interface {
            home,
            values,
            unions,
            aliases,
            binops,
        })
    }

    pub fn dependency(&mut self) -> Option<DependencyInterface> {
        (self.u8()? == VERSION).then_some(())?;
        match self.u8()? {
            0 => Some(DependencyInterface::Public(self.interface_body()?)),
            1 => Some(DependencyInterface::Private {
                home: self.str()?,
                unions: self.map(Decoder::union)?,
                aliases: self.map(Decoder::alias)?,
            }),
            _ => None,
        }
    }

    fn annotation(&mut self) -> Option<Annotation> {
        Some(Annotation {
            free: self.strs()?,
            ty: self.ty()?,
        })
    }

    fn alias(&mut self) -> Option<Alias> {
        Some(Alias {
            vars: self.strs()?,
            ty: self.ty()?,
        })
    }

    fn union(&mut self) -> Option<Union> {
        let vars = self.strs()?;
        let n = self.len()?;
        let ctors = (0..n)
            .map(|_| {
                let name = self.str()?;
                let index = self.len_value()?;
                let count = self.len()?;
                let args = (0..count).map(|_| self.ty()).collect::<Option<_>>()?;
                Some(Ctor { name, index, args })
            })
            .collect::<Option<_>>()?;
        let count = self.len_value()?;
        let opts = match self.u8()? {
            0 => CtorOpts::Normal,
            1 => CtorOpts::Enum,
            2 => CtorOpts::Unbox,
            _ => return None,
        };
        Some(Union {
            vars,
            ctors,
            count,
            opts,
        })
    }

    /// A number that counts something elsewhere, not bytes here.
    fn len_value(&mut self) -> Option<usize> {
        usize::try_from(self.u64()?).ok()
    }

    fn home(&mut self) -> Option<Home> {
        Some(Home {
            package: self.str()?,
            module: self.str()?,
        })
    }

    fn ty(&mut self) -> Option<Type> {
        stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || self.ty_())
    }

    fn ty_(&mut self) -> Option<Type> {
        Some(match self.u8()? {
            0 => Type::Lambda(Box::new(self.ty()?), Box::new(self.ty()?)),
            1 => Type::Var(self.str()?),
            2 => {
                let home = self.home()?;
                let name = self.str()?;
                let n = self.len()?;
                let args = (0..n).map(|_| self.ty()).collect::<Option<_>>()?;
                Type::Type(home, name, args)
            }
            3 => {
                let fields = self.map(|d| {
                    let order = u16::try_from(d.u64()?).ok()?;
                    Some(FieldType { order, ty: d.ty()? })
                })?;
                let ext = match self.u8()? {
                    0 => None,
                    1 => Some(self.str()?),
                    _ => return None,
                };
                Type::Record(fields, ext)
            }
            4 => Type::Unit,
            5 => {
                let a = self.ty()?;
                let b = self.ty()?;
                let c = match self.u8()? {
                    0 => None,
                    1 => Some(Box::new(self.ty()?)),
                    _ => return None,
                };
                Type::Tuple(Box::new(a), Box::new(b), c)
            }
            6 => {
                let home = self.home()?;
                let name = self.str()?;
                let n = self.len()?;
                let args = (0..n)
                    .map(|_| Some((self.str()?, self.ty()?)))
                    .collect::<Option<_>>()?;
                let aliased = match self.u8()? {
                    0 => AliasType::Holey(Box::new(self.ty()?)),
                    1 => AliasType::Filled(Box::new(self.ty()?)),
                    _ => return None,
                };
                Type::Alias(home, name, args, aliased)
            }
            _ => return None,
        })
    }
}
