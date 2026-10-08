//! Package names as upstream reads them (`Elm/Package.hs`):
//! `author/project`, the author alphanumeric and the project lowercase,
//! each with single dashes between and under 256 bytes. The rules and
//! their errors are the same in every Compat; only the cursor differs.

use std::fmt;

use nomi_base::{Compat, suggest};

use crate::Version;
use crate::json::{Cursor, Position};
use crate::scan::Scan;

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

    pub fn core() -> Name {
        Name::known("elm", "core")
    }

    pub fn json() -> Name {
        Name::known("elm", "json")
    }

    pub fn browser() -> Name {
        Name::known("elm", "browser")
    }

    pub fn html() -> Name {
        Name::known("elm", "html")
    }

    pub fn virtual_dom() -> Name {
        Name::known("elm", "virtual-dom")
    }

    pub fn author(&self) -> &str {
        &self.author
    }

    pub fn project(&self) -> &str {
        &self.project
    }

    /// `isKernel`: published by `elm` or `elm-explorations`, so allowed
    /// kernel code.
    pub fn is_kernel(&self) -> bool {
        is_official(&self.author)
    }

    /// `parser`, as both `decoder` and `keyDecoder` use it: a name and
    /// nothing after it, failing with the position upstream reports.
    pub fn parse(bytes: &[u8], start: Cursor, compat: Compat) -> Result<Name, Position> {
        let mut scan = Scan::new(bytes, start, compat);
        let name = Name::scan(&mut scan).map_err(Cursor::position)?;
        scan.finish().map_err(Cursor::position)?;
        Ok(name)
    }

    /// `Deps.Registry.newPkgParser`: a release as the registry lists new
    /// ones, `author/project@version`, and nothing after it.
    pub fn parse_release(
        bytes: &[u8],
        start: Cursor,
        compat: Compat,
    ) -> Result<(Name, Version), Position> {
        let mut scan = Scan::new(bytes, start, compat);
        let release = (|| {
            let name = Name::scan(&mut scan)?;
            scan.byte(b'@')?;
            let version = Version::scan(&mut scan)?;
            scan.finish()?;
            Ok((name, version))
        })();
        release.map_err(Cursor::position)
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
            let author = if is_official(&name.author) {
                0
            } else {
                suggest::distance(self.author.as_bytes(), name.author.as_bytes())
            };
            author + suggest::distance(self.project.as_bytes(), name.project.as_bytes())
        };
        let mut names: Vec<_> = names.into_iter().map(|n| (distance(n), n)).collect();
        names.sort_by_key(|&(d, _)| d);
        names.into_iter().take(4).map(|(_, n)| n).collect()
    }
}

/// `toChars`, as `encode` writes it.
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.author, self.project)
    }
}

fn is_official(author: &str) -> bool {
    author == "elm" || author == "elm-explorations"
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
