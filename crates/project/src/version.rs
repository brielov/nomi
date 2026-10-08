//! Package versions as upstream keeps them (`Elm/Version.hs`). 0.19.1 and
//! 0.19.2 hold three 16-bit numbers; 0.19.3 packs one 64-bit word with 21
//! bits for the major and minor numbers and 22 for the patch, so parsing
//! wraps or truncates each number to its width and bumping carries
//! between them as upstream's arithmetic does.

use std::fmt;

use nomi_base::Compat;

use crate::json::{Cursor, Position};
use crate::scan::Scan;

/// A version, each number already within its Compat's width, so the
/// derived order is upstream's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// 0.19.3's field widths.
const MAJOR_BITS: u32 = 21;
const MINOR_BITS: u32 = 21;
const PATCH_BITS: u32 = 22;

impl Version {
    pub const ONE: Version = Version::new(1, 0, 0);

    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// `toVersion`: the numbers as `compat` stores them.
    pub fn wrapped(compat: Compat, major: u32, minor: u32, patch: u32) -> Version {
        match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => Version::new(
                u32::from(major as u16),
                u32::from(minor as u16),
                u32::from(patch as u16),
            ),
            Compat::V0_19_3 => Version::new(
                low_bits(major, MAJOR_BITS),
                low_bits(minor, MINOR_BITS),
                low_bits(patch, PATCH_BITS),
            ),
        }
    }

    /// `max`: the largest major version, the upper end of `anything`.
    pub fn max(compat: Compat) -> Version {
        Version::wrapped(compat, u32::MAX, 0, 0)
    }

    pub fn bump_patch(self, compat: Compat) -> Version {
        match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => {
                Version::wrapped(compat, self.major, self.minor, self.patch + 1)
            }
            Compat::V0_19_3 => Version::unpack(self.pack().wrapping_add(1)),
        }
    }

    pub fn bump_minor(self, compat: Compat) -> Version {
        match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => {
                Version::wrapped(compat, self.major, self.minor + 1, 0)
            }
            Compat::V0_19_3 => {
                let without_patch = self.pack() & !((1 << PATCH_BITS) - 1);
                Version::unpack(without_patch.wrapping_add(1 << PATCH_BITS))
            }
        }
    }

    pub fn bump_major(self, compat: Compat) -> Version {
        Version::wrapped(compat, self.major.wrapping_add(1), 0, 0)
    }

    fn pack(self) -> u64 {
        u64::from(self.major) << (MINOR_BITS + PATCH_BITS)
            | u64::from(self.minor) << PATCH_BITS
            | u64::from(self.patch)
    }

    fn unpack(word: u64) -> Version {
        Version::new(
            (word >> (MINOR_BITS + PATCH_BITS)) as u32,
            low_bits((word >> PATCH_BITS) as u32, MINOR_BITS),
            low_bits(word as u32, PATCH_BITS),
        )
    }

    /// `parser`: `major.minor.patch`, failing where it stops making sense.
    pub(crate) fn scan(scan: &mut Scan<'_>) -> Result<Version, Cursor> {
        let major = number(scan)?;
        scan.byte(b'.')?;
        let minor = number(scan)?;
        scan.byte(b'.')?;
        let patch = number(scan)?;
        Ok(Version::wrapped(scan.compat(), major, minor, patch))
    }

    /// `decoder`'s string parser: a version and nothing after it, failing
    /// with the position upstream reports.
    pub fn parse(bytes: &[u8], start: Cursor, compat: Compat) -> Result<Version, Position> {
        let mut scan = Scan::new(bytes, start, compat);
        let version = Version::scan(&mut scan).map_err(Cursor::position)?;
        scan.finish().map_err(Cursor::position)?;
        Ok(version)
    }
}

/// `toChars`, as `encode` writes it.
impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn low_bits(n: u32, bits: u32) -> u32 {
    n & ((1 << bits) - 1)
}

/// `numberParser`: a zero alone, or digits not starting with one, summed
/// in 32 bits. A leading zero ends the number, so `01` fails on the `1`.
fn number(scan: &mut Scan<'_>) -> Result<u32, Cursor> {
    match scan.peek() {
        Some(b'0') => {
            scan.advance(1);
            Ok(0)
        }
        Some(digit @ b'1'..=b'9') => {
            let mut total = u32::from(digit - b'0');
            scan.advance(1);
            while let Some(digit @ b'0'..=b'9') = scan.peek() {
                total = total.wrapping_mul(10).wrapping_add(u32::from(digit - b'0'));
                scan.advance(1);
            }
            Ok(total)
        }
        _ => Err(scan.cursor()),
    }
}
