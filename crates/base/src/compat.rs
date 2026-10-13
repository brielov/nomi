//! The upstream release whose observable behavior nomi emulates. It comes
//! from an application's `elm-version`, which upstream requires to equal
//! the compiler's version (`Elm/Details.hs`, `verifyApp`).

use std::cmp::Ordering;
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

    /// What the release adds to a position it keeps where it shows one to
    /// editors, in `--report=json` regions and in the regions generated
    /// code passes `Debug.todo`: 1 from 0.19.3 (`toEditorRowCol`), and
    /// nothing before.
    pub fn editor_shift(self) -> u32 {
        self.first_json_position() - self.first_position()
    }

    /// A row or column as the release keeps it: 0.19.1's 16 bits wrap
    /// it (see [`Compat::column_wrap`]).
    pub fn wrap(self, n: u32) -> u32 {
        self.column_wrap().map_or(n, |wrap| n % wrap)
    }

    /// An editor's row or column, counted from 1, as the release keeps
    /// it: counted from its [`Compat::first_position`], and wrapped.
    pub fn keep(self, from_1: u32) -> u32 {
        self.wrap(from_1.wrapping_sub(1).wrapping_add(self.first_position()))
    }

    /// How the release orders names, wherever an order shows: as bytes,
    /// and from 0.19.3 shorter names first (`compareFast`).
    pub fn compare_names(self, a: &str, b: &str) -> Ordering {
        match self {
            Compat::V0_19_1 | Compat::V0_19_2 => a.cmp(b),
            Compat::V0_19_3 => (a.len(), a).cmp(&(b.len(), b)),
        }
    }

    /// `names` in the release's order, stably.
    pub fn sorted_names<'n, S: AsRef<str> + ?Sized>(
        self,
        names: impl IntoIterator<Item = &'n S>,
    ) -> Vec<&'n S> {
        let mut names: Vec<&S> = names.into_iter().collect();
        names.sort_by(|a, b| self.compare_names(a.as_ref(), b.as_ref()));
        names
    }

    /// The release's version, as `elm.json` writes it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Compat::V0_19_1 => "0.19.1",
            Compat::V0_19_2 => "0.19.2",
            Compat::V0_19_3 => "0.19.3",
        }
    }
}

impl fmt::Display for Compat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A version nomi doesn't emulate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownCompat(String);

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
            .find(|c| c.as_str() == text)
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

    #[test]
    fn names_order_by_length_from_0_19_3() {
        let names = ["bb", "a", "ab", "c"];
        let sorted = |compat: Compat| compat.sorted_names(names.iter().copied()).concat();
        assert_eq!(sorted(Compat::V0_19_2), "aabbbc");
        assert_eq!(sorted(Compat::V0_19_3), "acabbb");
        assert_eq!(Compat::V0_19_1.compare_names("b", "ab"), Ordering::Greater);
        assert_eq!(Compat::V0_19_3.compare_names("b", "ab"), Ordering::Less);
    }

    #[test]
    fn positions_are_kept_as_each_release_keeps_them() {
        assert_eq!(Compat::V0_19_1.keep(1), 1);
        assert_eq!(Compat::V0_19_1.keep(65_537), 1);
        assert_eq!(Compat::V0_19_1.keep(0), 0);
        assert_eq!(Compat::V0_19_2.keep(1), 0);
        assert_eq!(Compat::V0_19_3.keep(65_537), 65_536);
        assert_eq!(Compat::V0_19_3.keep(0), u32::MAX);
        let shifts = Compat::ALL.map(Compat::editor_shift);
        assert_eq!(shifts, [0, 0, 1]);
    }
}
