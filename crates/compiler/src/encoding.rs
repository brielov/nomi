//! The bytes of interfaces: tags as one byte, lengths as LEB128, other
//! numbers as little-endian fixed widths, maps in key order. A string is
//! written once, its length then its UTF-8, and named by its place after
//! that; an alias body, which every mention of the alias shares, likewise.
//! Both are told apart by value, so equal interfaces still encode alike.
//! Decoding takes nothing on trust: any malformed input is `None`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use nomi_base::grow_large;

use crate::canonical::{
    Alias, AliasType, Annotation, Ctor, CtorOpts, FieldType, Home, Type, Union,
};
use crate::hir::{Associativity, Fixity};
use crate::interface::{self, Binop, DependencyInterface, Interface};

/// Bumped whenever the encoding changes, so old bytes never decode as new.
const VERSION: u8 = 2;

/// Each fieldless enum's values, by the tag that encodes them.
const ASSOCIATIVITIES: [Associativity; 3] = [
    Associativity::Left,
    Associativity::Non,
    Associativity::Right,
];
const CTOR_OPTS: [CtorOpts; 3] = [CtorOpts::Normal, CtorOpts::Enum, CtorOpts::Unbox];

/// Each enum's variants, by the tag that encodes them; the encoder's
/// matches give the same tags.
const UNIONS: [fn(Union) -> interface::Union; 3] = [
    interface::Union::Open,
    interface::Union::Closed,
    interface::Union::Private,
];
const ALIASES: [fn(Alias) -> interface::Alias; 2] =
    [interface::Alias::Public, interface::Alias::Private];
const ALIAS_TYPES: [fn(Arc<Type>) -> AliasType; 2] = [AliasType::Holey, AliasType::Filled];

#[derive(Default)]
pub(crate) struct Encoder {
    out: Vec<u8>,
    /// Each string written, by its place.
    strings: HashMap<String, usize>,
    /// Each alias body written, by its place, and by where it lives.
    bodies: HashMap<Arc<Type>, usize>,
    body_at: HashMap<usize, usize>,
}

impl Encoder {
    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }

    fn u8(&mut self, n: u8) {
        self.out.push(n);
    }

    fn u64(&mut self, n: u64) {
        self.out.extend_from_slice(&n.to_le_bytes());
    }

    fn len(&mut self, n: usize) {
        let mut n = n as u64;
        while n >= 0x80 {
            self.out.push(n as u8 | 0x80);
            n >>= 7;
        }
        self.out.push(n as u8);
    }

    /// A string: 0 and the string the first time, then one more than its
    /// place.
    fn str(&mut self, s: &str) {
        if let Some(&place) = self.strings.get(s) {
            self.len(place + 1);
            return;
        }
        self.len(0);
        self.len(s.len());
        self.out.extend_from_slice(s.as_bytes());
        let place = self.strings.len();
        self.strings.insert(s.to_owned(), place);
    }

    /// An alias body as [`Encoder::str`] writes a string, its place given
    /// once it is written.
    fn body(&mut self, body: &Arc<Type>) {
        let at = Arc::as_ptr(body) as usize;
        let known = (self.body_at.get(&at).copied()).or_else(|| self.bodies.get(body).copied());
        if let Some(place) = known {
            self.body_at.insert(at, place);
            self.len(place + 1);
            return;
        }
        self.len(0);
        self.ty(body);
        let place = self.bodies.len();
        self.bodies.insert(body.clone(), place);
        self.body_at.insert(at, place);
    }

    /// `value`, by its tag in `table`.
    fn tag<T: Copy + PartialEq>(&mut self, table: &[T], value: T) {
        let tag = table.iter().position(|t| *t == value).expect("a tag");
        self.u8(u8::try_from(tag).expect("a small table"));
    }

    /// 0 for nothing, or 1 and the value.
    fn option<T: ?Sized>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Encoder, &T)) {
        match value {
            None => self.u8(0),
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
        }
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

    pub(crate) fn interface(&mut self, i: &Interface) {
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
            e.tag(&ASSOCIATIVITIES, b.fixity.associativity);
            e.u64(b.fixity.precedence as u64);
        });
    }

    pub(crate) fn dependency(&mut self, d: &DependencyInterface) {
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
        self.tag(&CTOR_OPTS, u.opts);
    }

    fn home(&mut self, home: &Home) {
        self.str(&home.package);
        self.str(&home.module);
    }

    fn ty(&mut self, t: &Type) {
        grow_large(|| self.ty_(t));
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
            Type::Type { home, name, args } => {
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
                self.option(ext.as_deref(), Encoder::str);
            }
            Type::Unit => self.u8(4),
            Type::Tuple(a, b, c) => {
                self.u8(5);
                self.ty(a);
                self.ty(b);
                self.option(c.as_deref(), Encoder::ty);
            }
            Type::Alias {
                home,
                name,
                args,
                aliased,
            } => {
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
                self.body(body);
            }
        }
    }
}

pub(crate) struct Decoder<'a> {
    rest: &'a [u8],
    strings: Vec<&'a str>,
    bodies: Vec<Arc<Type>>,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Decoder<'a> {
        Decoder {
            rest: bytes,
            strings: Vec::new(),
            bodies: Vec::new(),
        }
    }

    /// Whether every byte was read.
    pub(crate) fn end(&self) -> bool {
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

    /// LEB128, as [`Encoder::len`] writes it.
    fn leb128(&mut self) -> Option<usize> {
        let mut n: u64 = 0;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            n |= u64::from(byte & 0x7f).checked_shl(shift)?;
            if byte & 0x80 == 0 {
                return usize::try_from(n).ok();
            }
        }
        None
    }

    /// A length, which can't exceed what is left to read.
    fn len(&mut self) -> Option<usize> {
        let n = self.leb128()?;
        (n <= self.rest.len()).then_some(n)
    }

    fn str(&mut self) -> Option<String> {
        Some(self.str_ref()?.to_owned())
    }

    fn str_ref(&mut self) -> Option<&'a str> {
        match self.leb128()? {
            0 => {
                let n = self.len()?;
                let s = std::str::from_utf8(self.take(n)?).ok()?;
                self.strings.push(s);
                Some(s)
            }
            place => self.strings.get(place - 1).copied(),
        }
    }

    fn body(&mut self) -> Option<Arc<Type>> {
        match self.leb128()? {
            0 => {
                let body = Arc::new(self.ty()?);
                self.bodies.push(body.clone());
                Some(body)
            }
            place => self.bodies.get(place - 1).cloned(),
        }
    }

    /// A value of `table`, by its tag.
    fn tag<T: Copy>(&mut self, table: &[T]) -> Option<T> {
        table.get(usize::from(self.u8()?)).copied()
    }

    /// Whether a value follows, as [`Encoder::option`] marks it.
    fn present(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
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
        let mut entries: Vec<(String, V)> = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let key = self.str()?;
            // Keys come in order, once each, as encoding writes them.
            if entries.last().is_some_and(|(last, _)| *last >= key) {
                return None;
            }
            let entry = value(self)?;
            entries.push((key, entry));
        }
        // Sorted already, so the map is built in one pass.
        Some(entries.into_iter().collect())
    }

    pub(crate) fn interface(&mut self) -> Option<Interface> {
        (self.u8()? == VERSION).then_some(())?;
        self.interface_body()
    }

    fn interface_body(&mut self) -> Option<Interface> {
        let home = self.str()?;
        let values = self.map(Decoder::annotation)?;
        let unions = self.map(|d| Some(d.tag(&UNIONS)?(d.union()?)))?;
        let aliases = self.map(|d| Some(d.tag(&ALIASES)?(d.alias()?)))?;
        let binops = self.map(|d| {
            Some(Binop {
                function: d.str()?,
                annotation: d.annotation()?,
                fixity: Fixity {
                    associativity: d.tag(&ASSOCIATIVITIES)?,
                    precedence: d.u64()? as i64,
                },
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

    pub(crate) fn dependency(&mut self) -> Option<DependencyInterface> {
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
            ty: self.ty()?.into(),
        })
    }

    fn alias(&mut self) -> Option<Alias> {
        Some(Alias {
            vars: self.strs()?,
            ty: self.ty()?.into(),
        })
    }

    fn union(&mut self) -> Option<Union> {
        let vars = self.strs()?;
        let n = self.len()?;
        let ctors = (0..n)
            .map(|_| {
                let name = self.str()?;
                // A place among the constructors, not a length of bytes.
                let index = self.leb128()?;
                let count = self.len()?;
                let args = (0..count).map(|_| self.ty()).collect::<Option<_>>()?;
                Some(Ctor { name, index, args })
            })
            .collect::<Option<_>>()?;
        Some(Union {
            vars,
            ctors,
            // How many constructors, not a length of bytes.
            count: self.leb128()?,
            opts: self.tag(&CTOR_OPTS)?,
        })
    }

    fn home(&mut self) -> Option<Home> {
        Some(Home {
            package: self.str_ref()?.into(),
            module: self.str_ref()?.into(),
        })
    }

    fn ty(&mut self) -> Option<Type> {
        grow_large(|| self.ty_())
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
                Type::Type { home, name, args }
            }
            3 => {
                let fields = self.map(|d| {
                    let order = u16::try_from(d.u64()?).ok()?;
                    Some(FieldType { order, ty: d.ty()? })
                })?;
                let ext = if self.present()? {
                    Some(self.str()?)
                } else {
                    None
                };
                Type::Record(fields, ext)
            }
            4 => Type::Unit,
            5 => {
                let a = self.ty()?;
                let b = self.ty()?;
                let c = if self.present()? {
                    Some(Box::new(self.ty()?))
                } else {
                    None
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
                let aliased = self.tag(&ALIAS_TYPES)?(self.body()?);
                Type::Alias {
                    home,
                    name,
                    args,
                    aliased,
                }
            }
            _ => return None,
        })
    }
}
