//! Where upstream's parsers think they are. 0.19.1 keeps the row and the
//! column in 16 bits each, starting at (1, 1), and both wrap
//! (`Parse/Primitives.hs`, `Row`, `Col`). 0.19.2 and 0.19.3 pack a
//! zero-based row and column into one 64-bit word, the column in the low
//! half, so a column past 2³² carries into the row.

use nomi_base::Compat;

/// A position in editor coordinates, starting at (1, 1), as upstream
/// reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub row: u32,
    pub col: u32,
}

/// The positions upstream records around a value, the end exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub start: Position,
    pub end: Position,
}

/// A parser's position, in the representation of one Compat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// 0.19.1: one-based and 16 bits wide.
    Short { row: u16, col: u16 },
    /// 0.19.2 and later: zero-based and packed, the row in the high half.
    Packed(u64),
}

impl Cursor {
    /// The start of a file.
    pub fn start(compat: Compat) -> Cursor {
        match compat {
            Compat::V0_19_1 => Cursor::Short { row: 1, col: 1 },
            Compat::V0_19_2 | Compat::V0_19_3 => Cursor::Packed(0),
        }
    }

    /// `n` columns to the right.
    #[must_use]
    pub fn slide(self, n: usize) -> Cursor {
        match self {
            // Word16 arithmetic: only the low 16 bits of `n` matter.
            Cursor::Short { row, col } => Cursor::Short {
                row,
                col: col.wrapping_add(n as u16),
            },
            Cursor::Packed(cur) => Cursor::Packed(cur.wrapping_add(n as u64)),
        }
    }

    /// The start of the next row.
    #[must_use]
    pub fn newline(self) -> Cursor {
        match self {
            Cursor::Short { row, .. } => Cursor::Short {
                row: row.wrapping_add(1),
                col: 1,
            },
            Cursor::Packed(cur) => Cursor::Packed(cur.wrapping_add(1 << 32) & !0xFFFF_FFFF),
        }
    }

    /// One row down, the column kept: 0.19.1's position for a backslash
    /// that ends the input inside a string.
    #[must_use]
    pub(crate) fn down(self) -> Cursor {
        match self {
            Cursor::Short { row, col } => Cursor::Short {
                row: row.wrapping_add(1),
                col,
            },
            Cursor::Packed(cur) => Cursor::Packed(cur.wrapping_add(1 << 32)),
        }
    }

    pub fn position(self) -> Position {
        match self {
            Cursor::Short { row, col } => Position {
                row: row.into(),
                col: col.into(),
            },
            Cursor::Packed(cur) => Position {
                row: ((cur >> 32) as u32).wrapping_add(1),
                col: (cur as u32).wrapping_add(1),
            },
        }
    }
}
