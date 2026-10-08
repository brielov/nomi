//! The upstream release whose observable behavior nomi emulates. It comes
//! from an application's `elm-version`, which upstream requires to equal
//! the compiler's version (`Elm/Details.hs`, `verifyApp`).

use std::fmt;
use std::str::FromStr;

/// One supported elm release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Compat {
    V0_19_1,
    V0_19_2,
    V0_19_3,
}

impl Compat {
    /// Every supported release, oldest first.
    pub const ALL: [Compat; 3] = [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3];

    /// The release a package built alone takes.
    pub const LATEST: Compat = Compat::V0_19_3;

    /// The modulus columns wrap at while parsing, if they do: 0.19.1 keeps
    /// rows and columns in 16 bits (`Parse/Primitives.hs`, `Row`, `Col`),
    /// so a token far enough right looks unindented.
    pub fn column_wrap(self) -> Option<u32> {
        match self {
            Compat::V0_19_1 => Some(1 << 16),
            Compat::V0_19_2 | Compat::V0_19_3 => None,
        }
    }

    /// What the release's own rows and columns count from: 0.19.1 keeps
    /// them from 1, and later releases from 0 (`Reporting/Annotation.hs`),
    /// which shows wherever a report prints a position it keeps.
    pub fn first_position(self) -> u32 {
        match self {
            Compat::V0_19_1 => 1,
            Compat::V0_19_2 | Compat::V0_19_3 => 0,
        }
    }

    /// What `--report=json` regions count from: 0.19.2 prints the
    /// positions it keeps, from 0, and 0.19.3 adds 1 (`toEditorRowCol`).
    pub fn first_json_position(self) -> u32 {
        match self {
            Compat::V0_19_1 | Compat::V0_19_3 => 1,
            Compat::V0_19_2 => 0,
        }
    }
}

impl fmt::Display for Compat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Compat::V0_19_1 => "0.19.1",
            Compat::V0_19_2 => "0.19.2",
            Compat::V0_19_3 => "0.19.3",
        })
    }
}

/// A version nomi doesn't emulate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownCompat(pub String);

impl fmt::Display for UnknownCompat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "elm {} is not a supported release", self.0)
    }
}

impl std::error::Error for UnknownCompat {}

impl FromStr for Compat {
    type Err = UnknownCompat;

    fn from_str(text: &str) -> Result<Compat, UnknownCompat> {
        Compat::ALL
            .into_iter()
            .find(|c| c.to_string() == text)
            .ok_or_else(|| UnknownCompat(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_round_trip() {
        for compat in Compat::ALL {
            assert_eq!(compat.to_string().parse(), Ok(compat));
        }
        assert!("0.19.0".parse::<Compat>().is_err());
    }
}
