//! Module names as `elm.json` holds them (`Elm/ModuleName.hs` in 0.19.1
//! and 0.19.2, `AST/Prim/Module.hs`'s `jsonDecodeName` in 0.19.3):
//! capitalized names joined by dots, the whole string and nothing else.
//!
//! A failure is where the name stops making sense. 0.19.1 and 0.19.2
//! count that column in characters and reject names of 256 bytes or more;
//! 0.19.3 counts bytes and has no limit, since its check is on the bytes
//! after the name, which `fromSnippet` rejects anyway.

use nomi_base::Compat;

use crate::json::{Cursor, Position};
use crate::scan::{self, Scan};

/// A module name's bytes. 0.19.1 assembles characters without checking
/// their UTF-8, so the bytes it accepts need not be UTF-8.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleName(Box<[u8]>);

impl ModuleName {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    pub fn parse(bytes: &[u8], start: Cursor, compat: Compat) -> Result<ModuleName, Position> {
        let mut scan = Scan::new(bytes, start, compat);
        let columns = |width: usize| match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => 1,
            Compat::V0_19_3 => width,
        };
        'segment: loop {
            let width = scan::upper(scan.rest(), compat);
            if width == 0 {
                return Err(scan.cursor().position());
            }
            scan.skip(width, columns(width));
            loop {
                let width = scan::inner(scan.rest(), compat);
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
        let too_long = compat != Compat::V0_19_3 && scan.pos() >= 256;
        if too_long {
            return Err(scan.cursor().position());
        }
        scan.finish().map_err(Cursor::position)?;
        Ok(ModuleName(bytes.into()))
    }
}
