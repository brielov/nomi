//! Version constraints as upstream reads and combines them
//! (`Elm/Constraint.hs`): `lower op v op upper`, nothing more.

use std::cmp::Ordering;
use std::fmt;

use nomi_base::Compat;

use crate::json::{Cursor, Position};
use crate::scan::Scan;
use crate::version::Version;

/// A range of versions, `lower lower_op v upper_op upper`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Constraint {
    pub lower: Version,
    pub lower_op: Op,
    pub upper_op: Op,
    pub upper: Version,
}

/// How a range's end compares with the versions in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Less,
    LessOrEqual,
}

impl Op {
    fn holds(self, a: Version, b: Version) -> bool {
        match self {
            Op::Less => a < b,
            Op::LessOrEqual => a <= b,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Op::Less => " < ",
            Op::LessOrEqual => " <= ",
        }
    }
}

/// Why a constraint string doesn't parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintError {
    /// The string stops being a constraint here.
    BadFormat(Position),
    /// The lower end is not below the upper one.
    InvalidRange(Version, Version),
}

impl Constraint {
    /// `lower lower_op v upper_op upper`.
    pub const fn new(lower: Version, lower_op: Op, upper_op: Op, upper: Version) -> Constraint {
        Constraint {
            lower,
            lower_op,
            upper_op,
            upper,
        }
    }

    /// `exactly`: `version` alone.
    pub fn exactly(version: Version) -> Constraint {
        Constraint::new(version, Op::LessOrEqual, Op::LessOrEqual, version)
    }

    /// `anything`: every version from 1.0.0 on.
    pub fn anything(compat: Compat) -> Constraint {
        Constraint::new(
            Version::ONE,
            Op::LessOrEqual,
            Op::LessOrEqual,
            Version::max(compat),
        )
    }

    /// `untilNextMajor`: `version` up to the next major one.
    pub fn until_next_major(version: Version, compat: Compat) -> Constraint {
        Constraint::new(
            version,
            Op::LessOrEqual,
            Op::Less,
            version.bump_major(compat),
        )
    }

    /// `untilNextMinor`: `version` up to the next minor one.
    pub fn until_next_minor(version: Version, compat: Compat) -> Constraint {
        Constraint::new(
            version,
            Op::LessOrEqual,
            Op::Less,
            version.bump_minor(compat),
        )
    }

    /// `satisfies`: whether `version` is in the range.
    pub fn satisfies(&self, version: Version) -> bool {
        self.lower_op.holds(self.lower, version) && self.upper_op.holds(version, self.upper)
    }

    /// `check`: whether `version` is below, within or above the range.
    pub fn check(&self, version: Version) -> Ordering {
        if !self.lower_op.holds(self.lower, version) {
            Ordering::Less
        } else if !self.upper_op.holds(version, self.upper) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }

    /// The versions both allow, if any. Like upstream, a range that
    /// excludes its own single version (`1.0.0 < v <= 1.0.0`) still counts.
    pub fn intersect(&self, other: &Constraint) -> Option<Constraint> {
        let stricter = |a: Op, b: Op| {
            if a == Op::Less || b == Op::Less {
                Op::Less
            } else {
                Op::LessOrEqual
            }
        };
        let (lower, lower_op) = match self.lower.cmp(&other.lower) {
            Ordering::Less => (other.lower, other.lower_op),
            Ordering::Equal => (self.lower, stricter(self.lower_op, other.lower_op)),
            Ordering::Greater => (self.lower, self.lower_op),
        };
        let (upper, upper_op) = match self.upper.cmp(&other.upper) {
            Ordering::Less => (self.upper, self.upper_op),
            Ordering::Equal => (self.upper, stricter(self.upper_op, other.upper_op)),
            Ordering::Greater => (other.upper, other.upper_op),
        };
        (lower <= upper).then_some(Constraint::new(lower, lower_op, upper_op, upper))
    }

    /// `expand`: the range stretched to take in `version`, up to the
    /// major version after it.
    pub fn expand(&self, version: Version, compat: Compat) -> Constraint {
        if version < self.lower {
            Constraint {
                lower: version,
                lower_op: Op::LessOrEqual,
                ..*self
            }
        } else if version > self.upper {
            Constraint {
                upper_op: Op::Less,
                upper: version.bump_major(compat),
                ..*self
            }
        } else {
            *self
        }
    }

    /// `decoder`'s string parser.
    pub fn parse(
        bytes: &[u8],
        start: Cursor,
        compat: Compat,
    ) -> Result<Constraint, ConstraintError> {
        let bad = |cursor: Cursor| ConstraintError::BadFormat(cursor.position());
        let mut scan = Scan::new(bytes, start, compat);
        let lower = Version::scan(&mut scan).map_err(bad)?;
        scan.byte(b' ').map_err(bad)?;
        let lower_op = op(&mut scan).map_err(bad)?;
        for byte in *b" v " {
            scan.byte(byte).map_err(bad)?;
        }
        let upper_op = op(&mut scan).map_err(bad)?;
        scan.byte(b' ').map_err(bad)?;
        let upper = Version::scan(&mut scan).map_err(bad)?;
        if lower >= upper {
            return Err(ConstraintError::InvalidRange(lower, upper));
        }
        scan.finish().map_err(bad)?;
        Ok(Constraint::new(lower, lower_op, upper_op, upper))
    }
}

/// `toChars`, as `encode` writes it.
impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}v{}{}",
            self.lower,
            self.lower_op.as_str(),
            self.upper_op.as_str(),
            self.upper
        )
    }
}

/// `parseOp`: `<`, then `=` if there is one.
fn op(scan: &mut Scan<'_>) -> Result<Op, Cursor> {
    scan.byte(b'<')?;
    if scan.byte(b'=').is_ok() {
        Ok(Op::LessOrEqual)
    } else {
        Ok(Op::Less)
    }
}
