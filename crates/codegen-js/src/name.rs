//! `Generate/JavaScript/Name.hs`: the names generated code uses: locals
//! kept clear of JavaScript's reserved words, globals prefixed with their
//! package and module, kernels' values, and the short names `--optimize`
//! gives record fields, skipping those that would be reserved words. All
//! three releases name alike.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::OnceLock;

use nomi_compiler::canonical::Home;

/// A JavaScript name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Name {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// `fromInt`: the `n`th short name.
    pub(crate) fn from_int(n: usize) -> Name {
        Name(int_to_ascii(n))
    }

    /// `fromLocal`: a local, `_` before it if it is reserved.
    pub(crate) fn from_local(name: &str) -> Name {
        if reserved().contains(name) {
            Name(format!("_{name}"))
        } else {
            Name(name.to_owned())
        }
    }

    /// `fromGlobal`: `$author$project$Module$name`.
    pub(crate) fn from_global(home: &Home, name: &str) -> Name {
        Name::in_home(home, "$", name)
    }

    /// `fromCycle`: a value of a cycle, defined lazily.
    pub(crate) fn from_cycle(home: &Home, name: &str) -> Name {
        Name::in_home(home, "$cyclic$", name)
    }

    /// `homeToBuilder`, `$author$project$Module` with `-` as `_` in the
    /// package and `.` as `$` in the module, then `infix` and `name`.
    fn in_home(home: &Home, infix: &str, name: &str) -> Name {
        let (author, project) = home.package.split_once('/').unwrap_or(("", &home.package));
        let mut out = String::with_capacity(
            home.package.len() + home.module.len() + infix.len() + name.len() + 2,
        );
        for (part, from, to) in [
            (author, '-', '_'),
            (project, '-', '_'),
            (&home.module, '.', '$'),
        ] {
            out.push('$');
            out.extend(part.chars().map(|c| if c == from { to } else { c }));
        }
        out.push_str(infix);
        out.push_str(name);
        Name(out)
    }

    /// `fromKernel`: `_Module_name`.
    pub(crate) fn from_kernel(home: &str, name: &str) -> Name {
        Name(format!("_{home}_{name}"))
    }

    /// `makeF`: the currying helper of `n` arguments.
    pub(crate) fn make_f(n: usize) -> Name {
        Name(format!("F{n}"))
    }

    /// `makeA`: the application helper of `n` arguments.
    pub(crate) fn make_a(n: usize) -> Name {
        Name(format!("A{n}"))
    }

    /// `makeLabel`.
    pub(crate) fn make_label(name: &str, index: usize) -> Name {
        Name(format!("{name}${index}"))
    }

    /// `makeTemp`.
    pub(crate) fn make_temp(name: &str) -> Name {
        Name(format!("$temp${name}"))
    }

    /// `dollar`.
    pub(crate) fn dollar() -> Name {
        Name("$".to_owned())
    }
}

const JS_RESERVED: [&str; 66] = [
    "do",
    "if",
    "in",
    "NaN",
    "int",
    "for",
    "new",
    "try",
    "var",
    "let",
    "null",
    "true",
    "eval",
    "byte",
    "char",
    "goto",
    "long",
    "case",
    "else",
    "this",
    "void",
    "with",
    "enum",
    "false",
    "final",
    "float",
    "short",
    "break",
    "catch",
    "throw",
    "while",
    "class",
    "const",
    "super",
    "yield",
    "double",
    "native",
    "throws",
    "delete",
    "return",
    "switch",
    "typeof",
    "export",
    "import",
    "public",
    "static",
    "boolean",
    "default",
    "finally",
    "extends",
    "package",
    "private",
    "Infinity",
    "abstract",
    "volatile",
    "function",
    "continue",
    "debugger",
    "undefined",
    "arguments",
    "transient",
    "interface",
    "protected",
    "instanceof",
    "implements",
    "synchronized",
];

const ELM_RESERVED: [&str; 16] = [
    "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A9",
];

/// `reservedNames`.
fn reserved() -> &'static BTreeSet<&'static str> {
    static RESERVED: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    RESERVED.get_or_init(|| JS_RESERVED.into_iter().chain(ELM_RESERVED).collect())
}

const NUM_START_BYTES: usize = 54;
const NUM_INNER_BYTES: usize = 64;

/// `toByte`: lower, upper, `_`, `$`, then digits.
fn to_byte(n: usize) -> char {
    match n {
        0..26 => char::from(b'a' + n as u8),
        26..52 => char::from(b'A' + (n - 26) as u8),
        52 => '_',
        53 => '$',
        _ => char::from(b'0' + (n - 54) as u8),
    }
}

/// `unsafeIntToAscii`: `n` in `width` characters, the first from the
/// start bytes and the rest from the inner ones.
fn unsafe_int_to_ascii(width: usize, n: usize) -> String {
    let mut chars = Vec::with_capacity(width);
    let mut n = n;
    for _ in 1..width {
        chars.push(to_byte(n % NUM_INNER_BYTES));
        n /= NUM_INNER_BYTES;
    }
    chars.push(to_byte(n));
    chars.into_iter().rev().collect()
}

/// `allBadFields`: by width, the reserved words of that width, each
/// renamed to one of the last names of the width, the largest word (as
/// upstream's `Set.foldr` reaches it first) the last name.
fn bad_fields() -> &'static BTreeMap<usize, BTreeMap<String, String>> {
    static BAD: OnceLock<BTreeMap<usize, BTreeMap<String, String>>> = OnceLock::new();
    BAD.get_or_init(|| {
        let words: BTreeSet<&str> = JS_RESERVED.into_iter().collect();
        let mut by_width: BTreeMap<usize, BTreeMap<String, String>> = BTreeMap::new();
        for word in words.into_iter().rev() {
            let width = word.len();
            let renamings = by_width.entry(width).or_default();
            // Past 11 characters upstream's `Int` overflows; the renaming
            // is then never reached, and only how many there are counts.
            let renamed = (NUM_INNER_BYTES.checked_pow(width as u32 - 1))
                .and_then(|p| p.checked_mul(NUM_START_BYTES))
                .map_or_else(
                    || word.to_owned(),
                    |max| unsafe_int_to_ascii(width, max - 1 - renamings.len()),
                );
            renamings.insert(word.to_owned(), renamed);
        }
        by_width
    })
}

/// `intToAscii`: one character for the first 53, skipping `$`, then
/// wider names, each width short of the reserved words it would spell.
/// Upstream takes the next list of renamings at each width, though the
/// list skips the widths without reserved words: no reserved word has 11
/// letters, so width 11 counts the 12-letter one against itself.
fn int_to_ascii(n: usize) -> String {
    if n < 53 {
        return to_byte(n).to_string();
    }
    let mut n = n - 53;
    let mut width = 2;
    let mut block = NUM_START_BYTES * NUM_INNER_BYTES;
    let mut lists = bad_fields().values();
    loop {
        if let Some(renamings) = lists.next() {
            let available = block - renamings.len();
            if n < available {
                let name = unsafe_int_to_ascii(width, n);
                return renamings.get(&name).cloned().unwrap_or(name);
            }
            n -= available;
        } else {
            if n < block {
                return unsafe_int_to_ascii(width, n);
            }
            n -= block;
        }
        width += 1;
        block *= NUM_INNER_BYTES;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_skip_dollar_and_reserved_words() {
        assert_eq!(int_to_ascii(0), "a");
        assert_eq!(int_to_ascii(25), "z");
        assert_eq!(int_to_ascii(26), "A");
        assert_eq!(int_to_ascii(52), "_");
        assert_eq!(int_to_ascii(53), "aa");
        assert_eq!(Name::from_local("var").as_str(), "_var");
        assert_eq!(Name::from_local("F2").as_str(), "_F2");
        assert_eq!(Name::from_local("value").as_str(), "value");
    }

    #[test]
    fn globals_name_their_package_and_module() {
        let home = Home {
            package: "elm-community/list-extra".into(),
            module: "List.Extra".into(),
        };
        assert_eq!(
            Name::from_global(&home, "find").as_str(),
            "$elm_community$list_extra$List$Extra$find"
        );
        assert_eq!(
            Name::from_kernel("Utils", "Tuple2").as_str(),
            "_Utils_Tuple2"
        );
    }
}
