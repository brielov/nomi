//! Optimized graphs as each release writes them. 0.19.1 and 0.19.2 use
//! `Data.Binary`: a tag byte per constructor, `Int`s as eight big-endian
//! bytes, names in a length byte and their UTF-8, strings and byte
//! strings after an `Int` length, lists, maps and sets after an `Int`
//! count. 0.19.3 uses its own `Bytes.Encode`, alike but little-endian,
//! with counts and string lengths in four bytes, byte strings' in eight,
//! tuples split into pairs and triples, and types written its own way.
//! Maps and sets come in their keys' order, which 0.19.3 changed for
//! names to length first.
//!
//! A character literal is its text, escapes kept, in 0.19.1, and the
//! character itself, as its UTF-8, later. A region is four `Word16`s,
//! rows and columns from 1, in 0.19.1, and later two `Word64`s, each a
//! row from 0 above a column from 0.
//!
//! Decoding is for comparing with elm's own files: it rebuilds what
//! encoding takes, and any malformed input is `None`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::{
    Choice, Chunk, Decider, Def, Destructor, DtPath, EffectsType, Expr, Global, GlobalGraph,
    LocalGraph, Main, Node, Path, Test,
};
use nomi_base::Compat;
use nomi_base::diagnostic::code::{Position, Region};

use crate::canonical::{AliasType, CtorOpts, FieldType, Home, Type};

/// `Binary.encode` of a module's graph, as `<Module>.elmo` holds it.
pub fn encode_local_graph(graph: &LocalGraph, compat: Compat) -> Vec<u8> {
    let mut out = Out::new(compat);
    out.maybe(graph.main.as_ref(), Out::main);
    out.nodes(&graph.nodes);
    out.fields(&graph.fields);
    out.bytes
}

/// `Binary.encode` of the dependencies' graph, as `o.dat` holds it.
pub fn encode_global_graph(graph: &GlobalGraph, compat: Compat) -> Vec<u8> {
    let mut out = Out::new(compat);
    out.nodes(&graph.nodes);
    out.fields(&graph.fields);
    out.bytes
}

/// The graph in a `<Module>.elmo`.
pub fn decode_local_graph(bytes: &[u8], compat: Compat) -> Option<LocalGraph> {
    let mut input = In::new(bytes, compat);
    let graph = LocalGraph {
        main: input.maybe(In::main)?.into(),
        nodes: input.nodes()?,
        fields: input.fields()?,
    };
    input.done().then_some(graph)
}

/// The graph in an `o.dat`.
pub fn decode_global_graph(bytes: &[u8], compat: Compat) -> Option<GlobalGraph> {
    let mut input = In::new(bytes, compat);
    let graph = GlobalGraph {
        nodes: input.nodes()?,
        fields: input.fields()?,
    };
    input.done().then_some(graph)
}

/// `Ord Name`: by bytes, or in 0.19.3 by length, then bytes
/// (`compareFast`).
pub fn names(compat: Compat, a: &str, b: &str) -> Ordering {
    if compat == Compat::V0_19_3 {
        a.len()
            .cmp(&b.len())
            .then_with(|| a.as_bytes().cmp(b.as_bytes()))
    } else {
        a.as_bytes().cmp(b.as_bytes())
    }
}

/// `Ord ModuleName.Canonical`: by module name, then package, by author,
/// then project, each by bytes.
pub fn homes(compat: Compat, a: &Home, b: &Home) -> Ordering {
    let package = |home: &Home| {
        let (author, project) = split(home);
        (author.as_bytes().to_vec(), project.as_bytes().to_vec())
    };
    names(compat, &a.module, &b.module).then_with(|| package(a).cmp(&package(b)))
}

/// `Ord Opt.Global`: by name, then home.
pub fn globals(compat: Compat, a: &Global, b: &Global) -> Ordering {
    names(compat, &a.name, &b.name).then_with(|| homes(compat, &a.home, &b.home))
}

/// A home's package, as author and project.
fn split(home: &Home) -> (&str, &str) {
    home.package.split_once('/').unwrap_or(("", &home.package))
}

/// 0.19.3's constraint on a type variable, from its name (`TypeVar.hs`,
/// `varFromString`).
fn constraint(name: &str) -> u8 {
    match name {
        _ if name.len() < 6 => 0,
        _ if name.starts_with("comparable") => 1,
        _ if name.starts_with("appendable") => 2,
        _ if name.starts_with("compappend") => 3,
        _ if name.starts_with("number") => 4,
        _ => 0,
    }
}

struct Out {
    bytes: Vec<u8>,
    compat: Compat,
}

impl Out {
    fn new(compat: Compat) -> Out {
        Out {
            bytes: Vec::new(),
            compat,
        }
    }

    fn v1(&self) -> bool {
        self.compat == Compat::V0_19_1
    }

    fn v3(&self) -> bool {
        self.compat == Compat::V0_19_3
    }

    /// `Binary Char`'s UTF-8, or 0.19.1's `ES.String`.
    fn chr(&mut self, c: &str) {
        if self.v1() {
            self.string(c.as_bytes());
        } else {
            self.bytes.extend_from_slice(c.as_bytes());
        }
    }

    fn u8(&mut self, n: u8) {
        self.bytes.push(n);
    }

    fn int(&mut self, n: i64) {
        if self.v3() {
            self.bytes.extend_from_slice(&n.to_le_bytes());
        } else {
            self.bytes.extend_from_slice(&n.to_be_bytes());
        }
    }

    /// `Index.ZeroBased`, and a jump's target: an `Int`.
    fn index(&mut self, n: usize) {
        self.int(n as i64);
    }

    /// A list's, map's or set's length.
    fn count(&mut self, n: usize) {
        if self.v3() {
            self.bytes.extend_from_slice(&(n as u32).to_le_bytes());
        } else {
            self.int(n as i64);
        }
    }

    fn word16(&mut self, n: u16) {
        if self.v3() {
            self.bytes.extend_from_slice(&n.to_le_bytes());
        } else {
            self.bytes.extend_from_slice(&n.to_be_bytes());
        }
    }

    fn bool(&mut self, b: bool) {
        self.u8(u8::from(b));
    }

    /// `putUnder256`.
    fn name(&mut self, name: &str) {
        self.u8(name.len() as u8);
        self.bytes.extend_from_slice(name.as_bytes());
    }

    /// An Elm string: `putVeryLong`, or 0.19.3's `encode32`.
    fn string(&mut self, bytes: &[u8]) {
        self.count(bytes.len());
        self.bytes.extend_from_slice(bytes);
    }

    /// `Binary ByteString`, or 0.19.3's `byteString64`.
    fn byte_string(&mut self, bytes: &[u8]) {
        if self.v3() {
            self.bytes
                .extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        } else {
            self.int(bytes.len() as i64);
        }
        self.bytes.extend_from_slice(bytes);
    }

    fn maybe<T>(&mut self, value: Option<&T>, put: impl FnOnce(&mut Out, &T)) {
        match value {
            None => self.u8(0),
            Some(value) => {
                self.u8(1);
                put(self, value);
            }
        }
    }

    fn list<T>(&mut self, items: &[T], mut put: impl FnMut(&mut Out, &T)) {
        self.count(items.len());
        for item in items {
            put(self, item);
        }
    }

    fn names(&mut self, names: &[String]) {
        self.list(names, |out, name| out.name(name));
    }

    fn name_set(&mut self, set: &BTreeSet<String>) {
        let mut sorted: Vec<&String> = set.iter().collect();
        sorted.sort_by(|a, b| names(self.compat, a, b));
        self.count(sorted.len());
        for name in sorted {
            self.name(name);
        }
    }

    fn name_map<T>(&mut self, map: &BTreeMap<String, T>, mut put: impl FnMut(&mut Out, &T)) {
        let mut sorted: Vec<(&String, &T)> = map.iter().collect();
        sorted.sort_by(|a, b| names(self.compat, a.0, b.0));
        self.count(sorted.len());
        for (name, value) in sorted {
            self.name(name);
            put(self, value);
        }
    }

    fn home(&mut self, home: &Home) {
        let (author, project) = split(home);
        self.name(author);
        self.name(project);
        self.name(&home.module);
    }

    fn global(&mut self, global: &Global) {
        self.home(&global.home);
        self.name(&global.name);
    }

    fn global_set(&mut self, set: &BTreeSet<Global>) {
        let mut sorted: Vec<&Global> = set.iter().collect();
        sorted.sort_by(|a, b| globals(self.compat, a, b));
        self.count(sorted.len());
        for global in sorted {
            self.global(global);
        }
    }

    fn nodes(&mut self, nodes: &BTreeMap<Global, Node>) {
        let mut sorted: Vec<(&Global, &Node)> = nodes.iter().collect();
        sorted.sort_by(|a, b| globals(self.compat, a.0, b.0));
        self.count(sorted.len());
        for (global, node) in sorted {
            self.global(global);
            self.node(node);
        }
    }

    fn fields(&mut self, fields: &BTreeMap<String, i64>) {
        self.name_map(fields, |out, n| out.int(*n));
    }

    fn main(&mut self, main: &Main) {
        match main {
            Main::Static => self.u8(0),
            Main::Dynamic { message, decoder } => {
                self.u8(1);
                self.ty(message);
                self.expr(decoder);
            }
        }
    }

    fn node(&mut self, node: &Node) {
        match node {
            Node::Define(expr, deps) => {
                self.u8(0);
                self.expr(expr);
                self.global_set(deps);
            }
            Node::DefineTailFunc { params, body, deps } => {
                self.u8(1);
                self.names(params);
                self.expr(body);
                self.global_set(deps);
            }
            Node::Ctor(index, arity) => {
                self.u8(2);
                self.index(*index);
                self.int(*arity);
            }
            Node::Enum(index) => {
                self.u8(3);
                self.index(*index);
            }
            Node::Box => self.u8(4),
            Node::Link(global) => {
                self.u8(5);
                self.global(global);
            }
            Node::Cycle {
                names,
                values,
                functions,
                deps,
            } => {
                self.u8(6);
                self.names(names);
                self.list(values, |out, (name, expr)| {
                    out.name(name);
                    out.expr(expr);
                });
                self.list(functions, Out::def);
                self.global_set(deps);
            }
            Node::Manager(effects) => {
                self.u8(7);
                self.u8(match effects {
                    EffectsType::Cmd => 0,
                    EffectsType::Sub => 1,
                    EffectsType::Fx => 2,
                });
            }
            Node::Kernel(chunks, deps) => {
                self.u8(8);
                self.list(chunks, Out::chunk);
                self.global_set(deps);
            }
            Node::PortIncoming(expr, deps) => {
                self.u8(9);
                self.expr(expr);
                self.global_set(deps);
            }
            Node::PortOutgoing(expr, deps) => {
                self.u8(10);
                self.expr(expr);
                self.global_set(deps);
            }
        }
    }

    fn chunk(&mut self, chunk: &Chunk) {
        match chunk {
            Chunk::Js(bytes) => {
                self.u8(0);
                self.byte_string(bytes);
            }
            Chunk::ElmVar(home, name) => {
                self.u8(1);
                self.home(home);
                self.name(name);
            }
            Chunk::JsVar { module, name } => {
                self.u8(2);
                self.name(module);
                self.name(name);
            }
            Chunk::ElmField(name) => {
                self.u8(3);
                self.name(name);
            }
            Chunk::JsField(n) => {
                self.u8(4);
                self.int(*n);
            }
            Chunk::JsEnum(n) => {
                self.u8(5);
                self.int(*n);
            }
            Chunk::Debug => self.u8(6),
            Chunk::Prod => self.u8(7),
        }
    }

    fn def(&mut self, def: &Def) {
        match def {
            Def::Def(name, expr) => {
                self.u8(0);
                self.name(name);
                self.expr(expr);
            }
            Def::TailDef { name, params, body } => {
                self.u8(1);
                self.name(name);
                self.names(params);
                self.expr(body);
            }
        }
    }

    fn path(&mut self, path: &Path) {
        match path {
            Path::Index(index, rest) => {
                self.u8(0);
                self.index(*index);
                self.path(rest);
            }
            Path::Field(name, rest) => {
                self.u8(1);
                self.name(name);
                self.path(rest);
            }
            Path::Unbox(rest) => {
                self.u8(2);
                self.path(rest);
            }
            Path::Root(name) => {
                self.u8(3);
                self.name(name);
            }
        }
    }

    fn dt_path(&mut self, path: &DtPath) {
        match path {
            DtPath::Index(index, rest) => {
                self.u8(0);
                self.index(*index);
                self.dt_path(rest);
            }
            DtPath::Unbox(rest) => {
                self.u8(1);
                self.dt_path(rest);
            }
            DtPath::Empty => self.u8(2),
        }
    }

    fn test(&mut self, test: &Test) {
        match test {
            Test::IsCtor {
                home,
                name,
                index,
                count,
                opts,
            } => {
                self.u8(0);
                self.home(home);
                self.name(name);
                self.index(*index);
                self.int(*count);
                self.ctor_opts(*opts);
            }
            Test::IsCons => self.u8(1),
            Test::IsNil => self.u8(2),
            Test::IsTuple => self.u8(3),
            Test::IsChr(c) => {
                self.u8(4);
                self.chr(c);
            }
            Test::IsStr(s) => {
                self.u8(5);
                self.string(s.as_bytes());
            }
            Test::IsInt(n) => {
                self.u8(6);
                self.int(*n);
            }
            Test::IsBool(b) => {
                self.u8(7);
                self.bool(*b);
            }
        }
    }

    fn ctor_opts(&mut self, opts: CtorOpts) {
        self.u8(match opts {
            CtorOpts::Normal => 0,
            CtorOpts::Enum => 1,
            CtorOpts::Unbox => 2,
        });
    }

    fn decider(&mut self, decider: &Decider<Choice>) {
        match decider {
            Decider::Leaf(choice) => {
                self.u8(0);
                match choice {
                    Choice::Inline(expr) => {
                        self.u8(0);
                        self.expr(expr);
                    }
                    Choice::Jump(target) => {
                        self.u8(1);
                        self.index(*target);
                    }
                }
            }
            Decider::Chain {
                tests,
                success,
                failure,
            } => {
                self.u8(1);
                self.list(tests, |out, (path, test)| {
                    out.dt_path(path);
                    out.test(test);
                });
                self.decider(success);
                self.decider(failure);
            }
            Decider::FanOut {
                path,
                tests,
                fallback,
            } => {
                self.u8(2);
                self.dt_path(path);
                self.list(tests, |out, (test, decider)| {
                    out.test(test);
                    out.decider(decider);
                });
                self.decider(fallback);
            }
        }
    }

    fn region(&mut self, region: &Region) {
        for Position { row, col } in [region.start, region.end] {
            if self.v1() {
                self.word16(row as u16);
                self.word16(col as u16);
            } else {
                let word = (u64::from(row) << 32) | u64::from(col);
                self.int(word as i64);
            }
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Bool(b) => {
                self.u8(0);
                self.bool(*b);
            }
            Expr::Chr(c) => {
                self.u8(1);
                self.chr(c);
            }
            Expr::Str(s) => {
                self.u8(2);
                self.string(s.as_bytes());
            }
            Expr::Int(n) => {
                self.u8(3);
                self.int(*n);
            }
            Expr::Float(f) => {
                self.u8(4);
                self.name(f);
            }
            Expr::VarLocal(name) => {
                self.u8(5);
                self.name(name);
            }
            Expr::VarGlobal(global) => {
                self.u8(6);
                self.global(global);
            }
            Expr::VarEnum(global, index) => {
                self.u8(7);
                self.global(global);
                self.index(*index);
            }
            Expr::VarBox(global) => {
                self.u8(8);
                self.global(global);
            }
            Expr::VarCycle(home, name) => {
                self.u8(9);
                self.home(home);
                self.name(name);
            }
            Expr::VarDebug {
                name,
                home,
                region,
                unhandled,
            } => {
                self.u8(10);
                self.name(name);
                self.home(home);
                self.region(region);
                self.maybe(unhandled.as_ref(), |out, name| out.name(name));
            }
            Expr::VarKernel { module, name } => {
                self.u8(11);
                self.name(module);
                self.name(name);
            }
            Expr::List(items) => {
                self.u8(12);
                self.list(items, Out::expr);
            }
            Expr::Function(args, body) => {
                self.u8(13);
                self.names(args);
                self.expr(body);
            }
            Expr::Call(func, args) => {
                self.u8(14);
                self.expr(func);
                self.list(args, Out::expr);
            }
            Expr::TailCall(name, args) => {
                self.u8(15);
                self.name(name);
                self.list(args, |out, (name, expr)| {
                    out.name(name);
                    out.expr(expr);
                });
            }
            Expr::If(branches, otherwise) => {
                self.u8(16);
                self.list(branches, |out, (condition, then)| {
                    out.expr(condition);
                    out.expr(then);
                });
                self.expr(otherwise);
            }
            Expr::Let(def, body) => {
                self.u8(17);
                self.def(def);
                self.expr(body);
            }
            Expr::Destruct(destructor, body) => {
                self.u8(18);
                self.name(&destructor.name);
                self.path(&destructor.path);
                self.expr(body);
            }
            Expr::Case {
                label,
                root,
                decider,
                jumps,
            } => {
                self.u8(19);
                self.name(label);
                self.name(root);
                self.decider(decider);
                self.list(jumps, |out, (target, expr)| {
                    out.index(*target);
                    out.expr(expr);
                });
            }
            Expr::Accessor(field) => {
                self.u8(20);
                self.name(field);
            }
            Expr::Access(record, field) => {
                self.u8(21);
                self.expr(record);
                self.name(field);
            }
            Expr::Update(record, fields) => {
                self.u8(22);
                self.expr(record);
                self.name_map(fields, Out::expr);
            }
            Expr::Record(fields) => {
                self.u8(23);
                self.name_map(fields, Out::expr);
            }
            Expr::Unit => self.u8(24),
            Expr::Tuple(a, b, c) if self.v3() => {
                self.u8(if c.is_some() { 26 } else { 25 });
                self.expr(a);
                self.expr(b);
                if let Some(c) = c {
                    self.expr(c);
                }
            }
            Expr::Tuple(a, b, c) => {
                self.u8(25);
                self.expr(a);
                self.expr(b);
                self.maybe(c.as_deref(), Out::expr);
            }
            Expr::Shader {
                source,
                attributes,
                uniforms,
            } => {
                self.u8(if self.v3() { 27 } else { 26 });
                self.byte_string(source);
                self.name_set(attributes);
                self.name_set(uniforms);
            }
        }
    }

    fn ty(&mut self, ty: &Type) {
        if self.v3() {
            return self.ty3(ty);
        }
        match ty {
            Type::Lambda(a, b) => {
                self.u8(0);
                self.ty(a);
                self.ty(b);
            }
            Type::Var(name) => {
                self.u8(1);
                self.name(name);
            }
            Type::Record(fields, ext) => {
                self.u8(2);
                self.name_map(fields, |out, field| {
                    out.word16(field.order);
                    out.ty(&field.ty);
                });
                self.maybe(ext.as_ref(), |out, name| out.name(name));
            }
            Type::Unit => self.u8(3),
            Type::Tuple(a, b, c) => {
                self.u8(4);
                self.ty(a);
                self.ty(b);
                self.maybe(c.as_deref(), Out::ty);
            }
            Type::Alias {
                home,
                name,
                args,
                aliased: body,
            } => {
                self.u8(5);
                self.home(home);
                self.name(name);
                self.list(args, |out, (name, ty)| {
                    out.name(name);
                    out.ty(ty);
                });
                match body {
                    AliasType::Holey(ty) => {
                        self.u8(0);
                        self.ty(ty);
                    }
                    AliasType::Filled(ty) => {
                        self.u8(1);
                        self.ty(ty);
                    }
                }
            }
            // The arity in the tag when it fits.
            Type::Type { home, name, args } if args.len() + 7 <= 255 => {
                self.u8((args.len() + 7) as u8);
                self.home(home);
                self.name(name);
                for arg in args {
                    self.ty(arg);
                }
            }
            Type::Type { home, name, args } => {
                self.u8(6);
                self.home(home);
                self.name(name);
                self.list(args, Out::ty);
            }
        }
    }

    /// 0.19.3's `Interface.eType`.
    fn ty3(&mut self, ty: &Type) {
        let var = |out: &mut Out, name: &str| {
            out.name(name);
            out.u8(constraint(name));
        };
        match ty {
            Type::Lambda(a, b) => {
                self.u8(0);
                self.ty3(a);
                self.ty3(b);
            }
            Type::Var(name) => {
                self.u8(1);
                var(self, name);
            }
            Type::Record(fields, ext) => {
                self.u8(2);
                self.name_map(fields, |out, field| {
                    out.word16(field.order);
                    out.ty3(&field.ty);
                });
                self.maybe(ext.as_ref(), |out, name| var(out, name));
            }
            Type::Unit => self.u8(3),
            Type::Tuple(a, b, None) => {
                self.u8(4);
                self.ty3(a);
                self.ty3(b);
            }
            Type::Tuple(a, b, Some(c)) => {
                self.u8(5);
                self.ty3(a);
                self.ty3(b);
                self.ty3(c);
            }
            Type::Alias {
                home,
                name,
                args,
                aliased: body,
            } => {
                self.u8(6);
                self.home(home);
                self.name(name);
                self.u8(args.len() as u8);
                for (arg, ty) in args {
                    var(self, arg);
                    self.ty3(ty);
                }
                let (tag, ty) = match body {
                    AliasType::Holey(ty) => (0, ty),
                    AliasType::Filled(ty) => (1, ty),
                };
                self.u8(tag);
                self.ty3(ty);
            }
            Type::Type { home, name, args } => {
                self.u8(7);
                self.home(home);
                self.name(name);
                self.u8(args.len() as u8);
                for arg in args {
                    self.ty3(arg);
                }
            }
        }
    }
}

/// A `Maybe` read from the bytes, kept apart from the reader's `None`,
/// which means they don't decode.
enum Maybe<T> {
    Nothing,
    Just(T),
}

impl<T> From<Maybe<T>> for Option<T> {
    fn from(maybe: Maybe<T>) -> Option<T> {
        match maybe {
            Maybe::Nothing => None,
            Maybe::Just(value) => Some(value),
        }
    }
}

struct In<'a> {
    bytes: &'a [u8],
    at: usize,
    compat: Compat,
    /// Each home read, by its bytes: a graph names few, many times.
    homes: HashMap<&'a [u8], Home>,
}

impl<'a> In<'a> {
    fn new(bytes: &'a [u8], compat: Compat) -> In<'a> {
        In {
            bytes,
            at: 0,
            compat,
            homes: HashMap::new(),
        }
    }

    fn v1(&self) -> bool {
        self.compat == Compat::V0_19_1
    }

    fn v3(&self) -> bool {
        self.compat == Compat::V0_19_3
    }

    fn chr(&mut self) -> Option<String> {
        if self.v1() {
            return self.string();
        }
        let width = match self.bytes.get(self.at)? {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        };
        String::from_utf8(self.take(width)?.to_vec()).ok()
    }

    fn done(&self) -> bool {
        self.at == self.bytes.len()
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let bytes = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn int(&mut self) -> Option<i64> {
        let bytes = self.take(8)?.try_into().ok()?;
        Some(if self.v3() {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        })
    }

    fn index(&mut self) -> Option<usize> {
        usize::try_from(self.int()?).ok()
    }

    fn count(&mut self) -> Option<usize> {
        if self.v3() {
            let bytes = self.take(4)?.try_into().ok()?;
            return usize::try_from(u32::from_le_bytes(bytes)).ok();
        }
        usize::try_from(self.int()?).ok()
    }

    fn word16(&mut self) -> Option<u16> {
        let bytes = self.take(2)?.try_into().ok()?;
        Some(if self.v3() {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn name(&mut self) -> Option<String> {
        let n = self.u8()?;
        String::from_utf8(self.take(usize::from(n))?.to_vec()).ok()
    }

    fn byte_string(&mut self) -> Option<Vec<u8>> {
        let n = if self.v3() {
            usize::try_from(u64::from_le_bytes(self.take(8)?.try_into().ok()?)).ok()?
        } else {
            self.count()?
        };
        Some(self.take(n)?.to_vec())
    }

    fn string(&mut self) -> Option<String> {
        let n = self.count()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn maybe<T>(&mut self, get: impl FnOnce(&mut Self) -> Option<T>) -> Option<Maybe<T>> {
        match self.u8()? {
            0 => Some(Maybe::Nothing),
            1 => Some(Maybe::Just(get(self)?)),
            _ => None,
        }
    }

    fn list<T>(&mut self, mut get: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
        let n = self.count()?;
        // Each item takes a byte at least, so a count past the input is
        // malformed, not a huge allocation.
        if n > self.bytes.len() - self.at {
            return None;
        }
        (0..n).map(|_| get(self)).collect()
    }

    fn names(&mut self) -> Option<Vec<String>> {
        self.list(In::name)
    }

    fn name_set(&mut self) -> Option<BTreeSet<String>> {
        Some(self.names()?.into_iter().collect())
    }

    fn name_map<T>(
        &mut self,
        mut get: impl FnMut(&mut Self) -> Option<T>,
    ) -> Option<BTreeMap<String, T>> {
        let pairs = self.list(|input| Some((input.name()?, get(input)?)))?;
        Some(pairs.into_iter().collect())
    }

    fn home(&mut self) -> Option<Home> {
        let start = self.at;
        let mut end = start;
        for _ in 0..3 {
            end += 1 + usize::from(*self.bytes.get(end)?);
        }
        let bytes: &'a [u8] = self.bytes.get(start..end)?;
        if let Some(home) = self.homes.get(bytes) {
            self.at = end;
            return Some(home.clone());
        }
        let author = self.name()?;
        let project = self.name()?;
        let home = Home {
            package: format!("{author}/{project}").into(),
            module: self.name()?.into(),
        };
        self.homes.insert(bytes, home.clone());
        Some(home)
    }

    fn global(&mut self) -> Option<Global> {
        Some(Global {
            home: self.home()?,
            name: self.name()?,
        })
    }

    fn global_set(&mut self) -> Option<BTreeSet<Global>> {
        Some(self.list(In::global)?.into_iter().collect())
    }

    fn nodes(&mut self) -> Option<BTreeMap<Global, Node>> {
        let pairs = self.list(|input| Some((input.global()?, input.node()?)))?;
        Some(pairs.into_iter().collect())
    }

    fn fields(&mut self) -> Option<BTreeMap<String, i64>> {
        self.name_map(In::int)
    }

    fn main(&mut self) -> Option<Main> {
        match self.u8()? {
            0 => Some(Main::Static),
            1 => Some(Main::Dynamic {
                message: self.ty()?,
                decoder: Box::new(self.expr()?),
            }),
            _ => None,
        }
    }

    fn node(&mut self) -> Option<Node> {
        Some(match self.u8()? {
            0 => Node::Define(self.expr()?, self.global_set()?),
            1 => Node::DefineTailFunc {
                params: self.names()?,
                body: self.expr()?,
                deps: self.global_set()?,
            },
            2 => Node::Ctor(self.index()?, self.int()?),
            3 => Node::Enum(self.index()?),
            4 => Node::Box,
            5 => Node::Link(self.global()?),
            6 => Node::Cycle {
                names: self.names()?,
                values: self.list(|input| Some((input.name()?, input.expr()?)))?,
                functions: self.list(In::def)?,
                deps: self.global_set()?,
            },
            7 => Node::Manager(match self.u8()? {
                0 => EffectsType::Cmd,
                1 => EffectsType::Sub,
                2 => EffectsType::Fx,
                _ => return None,
            }),
            8 => Node::Kernel(self.list(In::chunk)?, self.global_set()?),
            9 => Node::PortIncoming(self.expr()?, self.global_set()?),
            10 => Node::PortOutgoing(self.expr()?, self.global_set()?),
            _ => return None,
        })
    }

    fn chunk(&mut self) -> Option<Chunk> {
        Some(match self.u8()? {
            0 => Chunk::Js(self.byte_string()?),
            1 => Chunk::ElmVar(self.home()?, self.name()?),
            2 => Chunk::JsVar {
                module: self.name()?,
                name: self.name()?,
            },
            3 => Chunk::ElmField(self.name()?),
            4 => Chunk::JsField(self.int()?),
            5 => Chunk::JsEnum(self.int()?),
            6 => Chunk::Debug,
            7 => Chunk::Prod,
            _ => return None,
        })
    }

    fn def(&mut self) -> Option<Def> {
        Some(match self.u8()? {
            0 => Def::Def(self.name()?, self.expr()?),
            1 => Def::TailDef {
                name: self.name()?,
                params: self.names()?,
                body: self.expr()?,
            },
            _ => return None,
        })
    }

    fn path(&mut self) -> Option<Path> {
        Some(match self.u8()? {
            0 => Path::Index(self.index()?, Box::new(self.path()?)),
            1 => Path::Field(self.name()?, Box::new(self.path()?)),
            2 => Path::Unbox(Box::new(self.path()?)),
            3 => Path::Root(self.name()?),
            _ => return None,
        })
    }

    fn dt_path(&mut self) -> Option<DtPath> {
        Some(match self.u8()? {
            0 => DtPath::Index(self.index()?, Arc::new(self.dt_path()?)),
            1 => DtPath::Unbox(Arc::new(self.dt_path()?)),
            2 => DtPath::Empty,
            _ => return None,
        })
    }

    fn test(&mut self) -> Option<Test> {
        Some(match self.u8()? {
            0 => Test::IsCtor {
                home: self.home()?,
                name: self.name()?,
                index: self.index()?,
                count: self.int()?,
                opts: self.ctor_opts()?,
            },
            1 => Test::IsCons,
            2 => Test::IsNil,
            3 => Test::IsTuple,
            4 => Test::IsChr(self.chr()?),
            5 => Test::IsStr(self.string()?),
            6 => Test::IsInt(self.int()?),
            7 => Test::IsBool(self.bool()?),
            _ => return None,
        })
    }

    fn ctor_opts(&mut self) -> Option<CtorOpts> {
        Some(match self.u8()? {
            0 => CtorOpts::Normal,
            1 => CtorOpts::Enum,
            2 => CtorOpts::Unbox,
            _ => return None,
        })
    }

    fn decider(&mut self) -> Option<Decider<Choice>> {
        Some(match self.u8()? {
            0 => Decider::Leaf(match self.u8()? {
                0 => Choice::Inline(self.expr()?),
                1 => Choice::Jump(self.index()?),
                _ => return None,
            }),
            1 => Decider::Chain {
                tests: self.list(|input| Some((input.dt_path()?, input.test()?)))?,
                success: Box::new(self.decider()?),
                failure: Box::new(self.decider()?),
            },
            2 => Decider::FanOut {
                path: self.dt_path()?,
                tests: self.list(|input| Some((input.test()?, input.decider()?)))?,
                fallback: Box::new(self.decider()?),
            },
            _ => return None,
        })
    }

    fn region(&mut self) -> Option<Region> {
        let mut position = || {
            if self.v1() {
                Some(Position {
                    row: u32::from(self.word16()?),
                    col: u32::from(self.word16()?),
                })
            } else {
                let word = self.int()? as u64;
                Some(Position {
                    row: u32::try_from(word >> 32).ok()?,
                    col: word as u32,
                })
            }
        };
        Some(Region {
            start: position()?,
            end: position()?,
        })
    }

    fn expr(&mut self) -> Option<Expr> {
        let boxed = |e: Expr| Box::new(e);
        Some(match self.u8()? {
            0 => Expr::Bool(self.bool()?),
            1 => Expr::Chr(self.chr()?),
            2 => Expr::Str(self.string()?),
            3 => Expr::Int(self.int()?),
            4 => Expr::Float(self.name()?),
            5 => Expr::VarLocal(self.name()?),
            6 => Expr::VarGlobal(self.global()?),
            7 => Expr::VarEnum(self.global()?, self.index()?),
            8 => Expr::VarBox(self.global()?),
            9 => Expr::VarCycle(self.home()?, self.name()?),
            10 => Expr::VarDebug {
                name: self.name()?,
                home: self.home()?,
                region: self.region()?,
                unhandled: self.maybe(In::name)?.into(),
            },
            11 => Expr::VarKernel {
                module: self.name()?,
                name: self.name()?,
            },
            12 => Expr::List(self.list(In::expr)?),
            13 => Expr::Function(self.names()?, boxed(self.expr()?)),
            14 => Expr::Call(boxed(self.expr()?), self.list(In::expr)?),
            15 => Expr::TailCall(
                self.name()?,
                self.list(|input| Some((input.name()?, input.expr()?)))?,
            ),
            16 => Expr::If(
                self.list(|input| Some((input.expr()?, input.expr()?)))?,
                boxed(self.expr()?),
            ),
            17 => Expr::Let(Box::new(self.def()?), boxed(self.expr()?)),
            18 => Expr::Destruct(
                Destructor {
                    name: self.name()?,
                    path: self.path()?,
                },
                boxed(self.expr()?),
            ),
            19 => Expr::Case {
                label: self.name()?,
                root: self.name()?,
                decider: Box::new(self.decider()?),
                jumps: self.list(|input| Some((input.index()?, input.expr()?)))?,
            },
            20 => Expr::Accessor(self.name()?),
            21 => Expr::Access(boxed(self.expr()?), self.name()?),
            22 => Expr::Update(boxed(self.expr()?), self.name_map(In::expr)?),
            23 => Expr::Record(self.name_map(In::expr)?),
            24 => Expr::Unit,
            25 if self.v3() => Expr::Tuple(boxed(self.expr()?), boxed(self.expr()?), None),
            26 if self.v3() => Expr::Tuple(
                boxed(self.expr()?),
                boxed(self.expr()?),
                Some(boxed(self.expr()?)),
            ),
            27 if self.v3() => self.shader()?,
            25 => Expr::Tuple(
                boxed(self.expr()?),
                boxed(self.expr()?),
                Option::from(self.maybe(In::expr)?).map(Box::new),
            ),
            26 => self.shader()?,
            _ => return None,
        })
    }

    fn shader(&mut self) -> Option<Expr> {
        Some(Expr::Shader {
            source: self.byte_string()?,
            attributes: self.name_set()?,
            uniforms: self.name_set()?,
        })
    }

    fn ty(&mut self) -> Option<Type> {
        if self.v3() {
            return self.ty3();
        }
        Some(match self.u8()? {
            0 => Type::Lambda(Box::new(self.ty()?), Box::new(self.ty()?)),
            1 => Type::Var(self.name()?),
            2 => Type::Record(
                self.name_map(|input| {
                    Some(FieldType {
                        order: input.word16()?,
                        ty: input.ty()?,
                    })
                })?,
                self.maybe(In::name)?.into(),
            ),
            3 => Type::Unit,
            4 => Type::Tuple(
                Box::new(self.ty()?),
                Box::new(self.ty()?),
                Option::from(self.maybe(In::ty)?).map(Box::new),
            ),
            5 => Type::Alias {
                home: self.home()?,
                name: self.name()?,
                args: self.list(|input| Some((input.name()?, input.ty()?)))?,
                aliased: match self.u8()? {
                    0 => AliasType::Holey(Arc::new(self.ty()?)),
                    1 => AliasType::Filled(Arc::new(self.ty()?)),
                    _ => return None,
                },
            },
            6 => Type::Type {
                home: self.home()?,
                name: self.name()?,
                args: self.list(In::ty)?,
            },
            n => {
                let home = self.home()?;
                let name = self.name()?;
                let args = (0..n - 7).map(|_| self.ty()).collect::<Option<_>>()?;
                Type::Type { home, name, args }
            }
        })
    }

    /// A type variable, whose constraint must be the one its name gives.
    fn var(&mut self) -> Option<String> {
        let name = self.name()?;
        (self.u8()? == constraint(&name)).then_some(name)
    }

    /// 0.19.3's `Interface.dType`.
    fn ty3(&mut self) -> Option<Type> {
        Some(match self.u8()? {
            0 => Type::Lambda(Box::new(self.ty3()?), Box::new(self.ty3()?)),
            1 => Type::Var(self.var()?),
            2 => Type::Record(
                self.name_map(|input| {
                    Some(FieldType {
                        order: input.word16()?,
                        ty: input.ty3()?,
                    })
                })?,
                self.maybe(In::var)?.into(),
            ),
            3 => Type::Unit,
            4 => Type::Tuple(Box::new(self.ty3()?), Box::new(self.ty3()?), None),
            5 => Type::Tuple(
                Box::new(self.ty3()?),
                Box::new(self.ty3()?),
                Some(Box::new(self.ty3()?)),
            ),
            6 => {
                let home = self.home()?;
                let name = self.name()?;
                let n = self.u8()?;
                let args = (0..n)
                    .map(|_| Some((self.var()?, self.ty3()?)))
                    .collect::<Option<_>>()?;
                let aliased = match self.u8()? {
                    0 => AliasType::Holey(Arc::new(self.ty3()?)),
                    1 => AliasType::Filled(Arc::new(self.ty3()?)),
                    _ => return None,
                };
                Type::Alias {
                    home,
                    name,
                    args,
                    aliased,
                }
            }
            7 => {
                let home = self.home()?;
                let name = self.name()?;
                let n = self.u8()?;
                let args = (0..n).map(|_| self.ty3()).collect::<Option<_>>()?;
                Type::Type { home, name, args }
            }
            _ => return None,
        })
    }
}
