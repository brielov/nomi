//! Package names as upstream reads them (`Elm/Package.hs`):
//! `author/project`, the author alphanumeric and the project lowercase,
//! each with single dashes between and under 256 bytes. The rules and
//! their errors are the same in every Compat; only the cursor differs.

use std::fmt;
use std::str::FromStr;

use nomi_base::{Compat, kernel, suggest};

use crate::Version;
use crate::json::{Cursor, Position};
use crate::scan::{self, Scan};

/// A valid package name, so ASCII. Ordered by author, then project, by
/// their bytes, as upstream orders them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name {
    author: Box<str>,
    project: Box<str>,
}

impl Name {
    /// A name from parts already known to be valid.
    fn known(author: &str, project: &str) -> Name {
        Name {
            author: author.into(),
            project: project.into(),
        }
    }

    /// `elm/core`.
    pub fn core() -> Name {
        Name::known("elm", "core")
    }

    /// `elm/json`.
    pub fn json() -> Name {
        Name::known("elm", "json")
    }

    /// `elm/browser`.
    pub fn browser() -> Name {
        Name::known("elm", "browser")
    }

    /// `elm/html`.
    pub fn html() -> Name {
        Name::known("elm", "html")
    }

    /// `elm/random`.
    pub fn random() -> Name {
        Name::known("elm", "random")
    }

    /// `elm/time`.
    pub fn time() -> Name {
        Name::known("elm", "time")
    }

    /// `nomi/test`, bundled with nomi.
    pub fn nomi_test() -> Name {
        Name::known(kernel::BUNDLED_AUTHOR, "test")
    }

    /// The part before the slash.
    pub fn author(&self) -> &str {
        &self.author
    }

    /// The part after the slash.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// `isKernel`: published by `elm` or `elm-explorations`, so allowed
    /// kernel code.
    pub fn is_kernel(&self) -> bool {
        kernel::is_author(&self.author)
    }

    /// Whether only nomi provides packages of this name: one by an author
    /// whose packages may hold kernel code, or by the author of those
    /// nomi bundles.
    pub fn is_reserved(&self) -> bool {
        self.is_kernel() || *self.author == *kernel::BUNDLED_AUTHOR
    }

    /// `parser`, as both `decoder` and `keyDecoder` use it: a name and
    /// nothing after it, failing with the position upstream reports.
    pub fn parse(bytes: &[u8], start: Cursor, compat: Compat) -> Result<Name, Position> {
        scan::whole(bytes, start, compat, Name::scan)
    }

    /// `Deps.Registry.newPkgParser`: a release as the registry lists new
    /// ones, `author/project@version`, and nothing after it.
    pub fn parse_release(
        bytes: &[u8],
        start: Cursor,
        compat: Compat,
    ) -> Result<(Name, Version), Position> {
        scan::whole(bytes, start, compat, |scan| {
            let name = Name::scan(scan)?;
            scan.byte(b'@')?;
            let version = Version::scan(scan)?;
            Ok((name, version))
        })
    }

    /// `parser`.
    fn scan(scan: &mut Scan<'_>) -> Result<Name, Cursor> {
        let author = part(scan, u8::is_ascii_alphanumeric, u8::is_ascii_alphanumeric)?;
        scan.byte(b'/')?;
        let project = part(scan, u8::is_ascii_lowercase, |b| {
            b.is_ascii_lowercase() || b.is_ascii_digit()
        })?;
        Ok(Name::known(author, project))
    }

    /// `nearbyNames`: the four of `names` closest to this one, an author
    /// of `elm` or `elm-explorations` counting as no distance at all.
    pub fn nearby<'n>(&self, names: impl IntoIterator<Item = &'n Name>) -> Vec<&'n Name> {
        let distance = |name: &Name| {
            let author = if kernel::is_author(&name.author) {
                0
            } else {
                suggest::distance(self.author.as_bytes(), name.author.as_bytes())
            };
            author + suggest::distance(self.project.as_bytes(), name.project.as_bytes())
        };
        let mut names: Vec<&Name> = names.into_iter().collect();
        names.sort_by_cached_key(|name| distance(name));
        names.truncate(4);
        names
    }
}

/// [`Name::parse`] of all of `text`, which every release reads alike.
impl FromStr for Name {
    type Err = Position;

    fn from_str(text: &str) -> Result<Name, Position> {
        let compat = Compat::LATEST;
        Name::parse(text.as_bytes(), Cursor::start(compat), compat)
    }
}

/// `toChars`, as `encode` writes it.
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.author, self.project)
    }
}

/// `parseName`: a good first byte, then good bytes with single dashes
/// between them. A dash may end the part unless it ends the string too,
/// as upstream only checks for a trailing dash there.
fn part<'a>(
    scan: &mut Scan<'a>,
    good_start: impl Fn(&u8) -> bool,
    good_inner: impl Fn(&u8) -> bool,
) -> Result<&'a str, Cursor> {
    let rest = scan.rest();
    if !rest.first().is_some_and(&good_start) {
        return Err(scan.cursor());
    }
    let mut len = 1;
    let mut after_dash = false;
    let good = loop {
        match rest.get(len) {
            None => break !after_dash,
            Some(b) if good_inner(b) => after_dash = false,
            Some(b'-') if after_dash => break false,
            Some(b'-') => after_dash = true,
            Some(_) => break true,
        }
        len += 1;
    };
    scan.advance(len);
    if good && len < 256 {
        Ok(std::str::from_utf8(&rest[..len]).expect("ASCII"))
    } else {
        Err(scan.cursor())
    }
}
