//! Module names as `elm.json` holds them (`Elm/ModuleName.hs` in 0.19.1
//! and 0.19.2, `AST/Prim/Module.hs`'s `jsonDecodeName` in 0.19.3):
//! capitalized names joined by dots, the whole string and nothing else.
//!
//! A failure is where the name stops making sense. 0.19.1 and 0.19.2
//! count that column in characters and reject names of 256 bytes or more;
//! 0.19.3 counts bytes and has no limit, since its check is on the bytes
//! after the name, which `fromSnippet` rejects anyway.

use std::cmp::Ordering;
use std::fmt;

use nomi_base::{Compat, utf8};

use crate::json::{Cursor, Position};
use crate::scan;

/// A module name's bytes. 0.19.1 assembles characters without checking
/// their UTF-8, so the bytes it accepts need not be UTF-8.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleName(Box<[u8]>);

impl ModuleName {
    /// A name another reader already checked, as the parser checks the
    /// names in a module's header and imports.
    pub fn new(name: &str) -> ModuleName {
        ModuleName(name.as_bytes().into())
    }

    /// The name's bytes, as `elm.json` has them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The name, if it is UTF-8.
    pub fn to_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// `isKernel`: one of the kernel's JavaScript modules, which only
    /// `elm/` and `elm-explorations/` packages may import.
    pub fn is_kernel(&self) -> bool {
        self.0.starts_with(b"Elm.Kernel.")
    }

    /// The order upstream keeps module names in, which decides the order
    /// of its maps and graphs.
    pub fn compare(&self, other: &ModuleName, compat: Compat) -> Ordering {
        compat.compare_name_bytes(&self.0, &other.0)
    }

    /// `jsonDecodeName`'s parser: a name and nothing after it, failing
    /// with the position upstream reports.
    pub(crate) fn parse(
        bytes: &[u8],
        start: Cursor,
        compat: Compat,
    ) -> Result<ModuleName, Position> {
        let columns = |width: usize| match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => 1,
            Compat::V0_19_3 => width,
        };
        scan::whole(bytes, start, compat, |scan| {
            'segment: loop {
                let width = utf8::upper_width(scan.rest(), compat);
                if width == 0 {
                    return Err(scan.cursor());
                }
                scan.skip(width, columns(width));
                loop {
                    let width = utf8::inner_width(scan.rest(), compat);
                    if width > 0 {
                        scan.skip(width, columns(width));
                    } else if scan.peek() == Some(b'.') {
                        scan.advance(1);
                        continue 'segment;
                    } else {
                        break 'segment;
                    }
                }
            }
            let too_long = match compat {
                Compat::V0_19_1 | Compat::V0_19_2 => scan.pos() >= 256,
                Compat::V0_19_3 => false,
            };
            if too_long { Err(scan.cursor()) } else { Ok(()) }
        })?;
        Ok(ModuleName(bytes.into()))
    }
}

/// `isKernel` of a name another reader already checked.
pub fn is_kernel(name: &str) -> bool {
    name.starts_with("Elm.Kernel.")
}

/// A kernel module's name without its `Elm.Kernel.` prefix.
pub fn kernel_short(name: &str) -> Option<&str> {
    name.strip_prefix("Elm.Kernel.")
}

impl fmt::Display for ModuleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}
