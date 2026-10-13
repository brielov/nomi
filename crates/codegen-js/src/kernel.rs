//! Kernel modules (`Elm/Kernel.hs`): a package's hand-written JavaScript,
//! a comment of Elm imports, then code in which `__` starts a tag:
//! `__$field` an Elm record field, `__1_NAME` an enumeration, `__name` a
//! field of the kernel's own objects, `__DEBUG` and `__PROD` the modes'
//! markers, and `__Module_value` a value the imports expose. A kernel
//! that doesn't parse is `None`, as upstream refuses to build it.

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::{Chunk, Global, GlobalGraph, Node};

/// `Content`: a kernel's imports and its code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub imports: Vec<Import>,
    pub chunks: Vec<Chunk>,
}

/// An import of the header comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub module: String,
    pub alias: Option<String>,
    pub exposing: Vec<String>,
}

/// `fromByteString`: the kernel `bytes` of `package`, whose dependencies
/// expose the modules in `foreigns`, by the package exposing each.
pub fn parse(package: &str, foreigns: &BTreeMap<String, String>, bytes: &[u8]) -> Option<Content> {
    let mut header = Header { bytes, at: 0 };
    let imports = header.imports()?;
    let table = var_table(package, foreigns, &imports)?;
    let chunks = chunks(&table, &bytes[header.at..])?;
    Some(Content { imports, chunks })
}

/// `countFields`: how often the code uses each Elm record field.
pub fn count_fields(chunks: &[Chunk]) -> BTreeMap<String, i64> {
    let mut fields = BTreeMap::new();
    for chunk in chunks {
        if let Chunk::ElmField(name) = chunk {
            *fields.entry(name.clone()).or_default() += 1;
        }
    }
    fields
}

/// `Opt.addKernel`: the kernel `short` (its module without
/// `Elm.Kernel.`) as a node of `graph`, needing what its code names. Its
/// field counts replace the graph's, as upstream's left-biased union does.
pub fn add_kernel(compat: Compat, graph: &mut GlobalGraph, short: &str, chunks: Vec<Chunk>) {
    let deps = (chunks.iter())
        .filter_map(|chunk| match chunk {
            Chunk::ElmVar(home, name) => Some(Global {
                home: home.clone(),
                name: name.clone(),
            }),
            Chunk::JsVar { module, .. } => Some(Global::kernel(compat, module)),
            _ => None,
        })
        .collect();
    graph.fields.extend(count_fields(&chunks));
    graph
        .nodes
        .insert(Global::kernel(compat, short), Node::Kernel(chunks, deps));
}

/// `toVarTable`: what each `__Prefix_name` tag stands for.
fn var_table(
    package: &str,
    foreigns: &BTreeMap<String, String>,
    imports: &[Import],
) -> Option<BTreeMap<String, Chunk>> {
    let mut table = BTreeMap::new();
    for import in imports {
        if let Some(kernel) = import.module.strip_prefix("Elm.Kernel.") {
            if import.alias.is_some() {
                return None;
            }
            for name in &import.exposing {
                let chunk = Chunk::JsVar {
                    module: kernel.to_owned(),
                    name: name.clone(),
                };
                table.insert(format!("{kernel}_{name}"), chunk);
            }
        } else {
            let home = Home {
                package: foreigns
                    .get(&import.module)
                    .map_or(package, String::as_str)
                    .into(),
                module: import.module.clone().into(),
            };
            let prefix = match &import.alias {
                Some(alias) => alias.clone(),
                None if import.module.contains('.') => return None,
                None => import.module.clone(),
            };
            for name in &import.exposing {
                let chunk = Chunk::ElmVar(home.clone(), name.clone());
                table.insert(format!("{prefix}_{name}"), chunk);
            }
        }
    }
    Some(table)
}

/// `chompChunks`: the code split at its tags.
fn chunks(table: &BTreeMap<String, Chunk>, code: &[u8]) -> Option<Vec<Chunk>> {
    let mut chunks = Vec::new();
    let mut fields: BTreeMap<String, i64> = BTreeMap::new();
    let mut enums: BTreeMap<u8, BTreeMap<String, i64>> = BTreeMap::new();
    let (mut pos, mut last) = (0, 0);
    while pos < code.len() {
        if code[pos] != b'_' {
            pos += char_width(code[pos]);
            continue;
        }
        if !(pos + 3 <= code.len() && code[pos + 1] == b'_') {
            pos += 1;
            continue;
        }
        chunks.push(Chunk::Js(code[last..pos].to_vec()));
        let tag = code[pos + 2];
        let end = inner_end(code, pos + 3);
        let name = |from: usize| String::from_utf8(code[from..end].to_vec()).ok();
        let chunk = match tag {
            b'$' => Chunk::ElmField(name(pos + 3)?),
            b'0'..=b'9' => {
                let enums = enums.entry(tag - b'0').or_default();
                let next = enums.len() as i64;
                Chunk::JsEnum(*enums.entry(name(pos + 2)?).or_insert(next))
            }
            b'a'..=b'z' => {
                let next = fields.len() as i64;
                Chunk::JsField(*fields.entry(name(pos + 2)?).or_insert(next))
            }
            _ => match name(pos + 2)?.as_str() {
                "DEBUG" => Chunk::Debug,
                "PROD" => Chunk::Prod,
                other => table.get(other)?.clone(),
            },
        };
        chunks.push(chunk);
        pos = end;
        last = end;
    }
    chunks.push(Chunk::Js(code[last..].to_vec()));
    Some(chunks)
}

/// `getCharWidth`: the bytes of the character `byte` starts.
fn char_width(byte: u8) -> usize {
    match byte {
        0x00..0xC0 => 1,
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        _ => 4,
    }
}

/// `Var.chompInnerChars`: past the letters, digits and underscores from
/// `pos`.
fn inner_end(code: &[u8], mut pos: usize) -> usize {
    while pos < code.len() {
        let rest = &code[pos..];
        let c = match std::str::from_utf8(&rest[..char_width(rest[0]).min(rest.len())]) {
            Ok(s) => s.chars().next().unwrap_or(' '),
            Err(_) => break,
        };
        if !(c.is_alphanumeric() || c == '_') {
            break;
        }
        pos += c.len_utf8();
    }
    pos
}

/// The header comment: `/*`, the imports, each on a line of its own, and
/// `*/`.
struct Header<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Header<'_> {
    fn imports(&mut self) -> Option<Vec<Import>> {
        self.word("/*")?;
        self.spaces()?;
        let mut imports = Vec::new();
        while self.fresh_line() && self.keyword("import") {
            self.spaces()?;
            let module = self.module_name()?;
            self.spaces()?;
            let alias = if self.keyword("as") {
                self.spaces()?;
                let alias = self.upper()?;
                self.spaces()?;
                Some(alias)
            } else {
                None
            };
            let exposing = if self.keyword("exposing") {
                self.spaces()?;
                let list = self.exposing()?;
                self.spaces()?;
                list
            } else {
                Vec::new()
            };
            imports.push(Import {
                module,
                alias,
                exposing,
            });
        }
        self.word("*/")?;
        Some(imports)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn word(&mut self, word: &str) -> Option<()> {
        self.bytes[self.at..]
            .starts_with(word.as_bytes())
            .then(|| self.at += word.len())
    }

    /// A word not followed by more of a name.
    fn keyword(&mut self, word: &str) -> bool {
        let rest = &self.bytes[self.at..];
        let next = rest.get(word.len()).copied();
        let ends = !next.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
        let found = rest.starts_with(word.as_bytes()) && ends;
        if found {
            self.at += word.len();
        }
        found
    }

    /// `Space.chomp`: spaces, newlines and comments; tabs are refused.
    fn spaces(&mut self) -> Option<()> {
        loop {
            match self.peek() {
                Some(b' ' | b'\n' | b'\r') => self.at += 1,
                Some(b'\t') => return None,
                Some(b'-') if self.bytes.get(self.at + 1) == Some(&b'-') => {
                    while self.peek().is_some_and(|b| b != b'\n') {
                        self.at += 1;
                    }
                }
                Some(b'{') if self.bytes.get(self.at + 1) == Some(&b'-') => {
                    self.at += 2;
                    let mut depth = 1;
                    while depth > 0 {
                        let rest = &self.bytes[self.at..];
                        if rest.is_empty() {
                            return None;
                        } else if rest.starts_with(b"{-") {
                            depth += 1;
                            self.at += 2;
                        } else if rest.starts_with(b"-}") {
                            depth -= 1;
                            self.at += 2;
                        } else {
                            self.at += 1;
                        }
                    }
                }
                _ => return Some(()),
            }
        }
    }

    /// `Space.checkFreshLine`: at the start of a line.
    fn fresh_line(&self) -> bool {
        self.at == 0 || self.bytes[self.at - 1] == b'\n'
    }

    fn name(&mut self, first: fn(u8) -> bool) -> Option<String> {
        let start = self.at;
        if !self.peek().is_some_and(first) {
            return None;
        }
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.at += 1;
        }
        String::from_utf8(self.bytes[start..self.at].to_vec()).ok()
    }

    fn upper(&mut self) -> Option<String> {
        self.name(|b| b.is_ascii_uppercase())
    }

    fn module_name(&mut self) -> Option<String> {
        let mut name = self.upper()?;
        while self.peek() == Some(b'.')
            && self
                .bytes
                .get(self.at + 1)
                .is_some_and(u8::is_ascii_uppercase)
        {
            self.at += 1;
            name.push('.');
            name.push_str(&self.upper()?);
        }
        Some(name)
    }

    /// An exposing list of plain names: no `(..)`, no operators.
    fn exposing(&mut self) -> Option<Vec<String>> {
        self.word("(")?;
        let mut names = Vec::new();
        loop {
            self.spaces()?;
            names.push(self.name(|b| b.is_ascii_alphabetic())?);
            self.spaces()?;
            match self.peek()? {
                b',' => self.at += 1,
                b')' => {
                    self.at += 1;
                    return Some(names);
                }
                _ => return None,
            }
        }
    }
}
