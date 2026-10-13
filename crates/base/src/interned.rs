//! Interned names.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{OnceLock, RwLock};

use rustc_hash::FxHashSet;

/// A name, a package's, a module's or a type's, interned: every `Text` of
/// the same name points at one string, kept for the life of the process,
/// so a clone copies a pointer, equality compares pointers, and a drop
/// does nothing. The pointer is thin, to where the string's own pointer
/// and length are kept, so a `Text` is one word: type variables hold
/// several. Names are few, so keeping them costs little. It orders and
/// hashes as its text, so nothing depends on where a name lives.
#[derive(Clone)]
#[expect(
    clippy::borrowed_box,
    reason = "a `&Box<str>` is one word, a `&str` two"
)]
pub struct Text(&'static Box<str>);

/// An interned name as the interner's sets hold it, found by its text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::borrowed_box,
    reason = "it holds the one-word pointer a `Text` holds"
)]
struct Interned(&'static Box<str>);

impl Borrow<str> for Interned {
    #[inline]
    fn borrow(&self) -> &str {
        self.0
    }
}

impl Text {
    /// The interned `name`.
    pub fn new(name: &str) -> Text {
        static NAMES: OnceLock<RwLock<FxHashSet<Interned>>> = OnceLock::new();
        // Each thread keeps the names it has seen, so threads interning at
        // once don't contend for the shared set.
        thread_local! {
            static SEEN: RefCell<FxHashSet<Interned>> = RefCell::default();
        }
        if let Some(interned) = SEEN.with_borrow(|seen| seen.get(name).copied()) {
            return Text(interned.0);
        }
        let names = NAMES.get_or_init(Default::default);
        let known = (names.read().unwrap_or_else(|e| e.into_inner()).get(name)).copied();
        let interned = known.unwrap_or_else(|| {
            let mut names = names.write().unwrap_or_else(|e| e.into_inner());
            if let Some(&interned) = names.get(name) {
                return interned;
            }
            let interned = Interned(Box::leak(Box::new(Box::from(name))));
            names.insert(interned);
            interned
        });
        SEEN.with_borrow_mut(|seen| seen.insert(interned));
        Text(interned.0)
    }

    /// The name's text, which lives as long as the process.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl Default for Text {
    fn default() -> Text {
        Text::new("")
    }
}

impl PartialEq for Text {
    #[inline]
    fn eq(&self, other: &Text) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    #[inline]
    fn partial_cmp(&self, other: &Text) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    #[inline]
    fn cmp(&self, other: &Text) -> Ordering {
        if self == other {
            Ordering::Equal
        } else {
            self.as_str().cmp(other.as_str())
        }
    }
}

impl Hash for Text {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl Deref for Text {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Text {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Text {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for Text {
    #[inline]
    fn from(s: String) -> Text {
        Text::new(&s)
    }
}

impl From<&str> for Text {
    #[inline]
    fn from(s: &str) -> Text {
        Text::new(s)
    }
}

impl From<&String> for Text {
    #[inline]
    fn from(s: &String) -> Text {
        Text::new(s)
    }
}

impl From<Text> for String {
    #[inline]
    fn from(t: Text) -> String {
        t.as_str().to_owned()
    }
}

impl PartialEq<str> for Text {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Text {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Text {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Text> for str {
    #[inline]
    fn eq(&self, other: &Text) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Text> for &str {
    #[inline]
    fn eq(&self, other: &Text) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<Text> for String {
    #[inline]
    fn eq(&self, other: &Text) -> bool {
        self.as_str() == other.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_one_string_however_it_is_interned() {
        let basics = Text::new("Basics");
        let again = Text::from("Basics".to_owned());
        assert!(std::ptr::eq(basics.as_str(), again.as_str()));
        assert_eq!(basics, again);
        assert_ne!(basics, Text::new("Maybe"));
        assert_eq!(size_of::<Text>(), size_of::<usize>());
    }

    #[test]
    fn names_order_and_hash_as_their_text() {
        assert!(Text::new("Maybe") > Text::new("Basics"));
        assert_eq!(Text::new("b").cmp(&Text::new("b")), Ordering::Equal);
        let set: FxHashSet<Text> = [Text::new("List")].into_iter().collect();
        assert!(set.contains("List"));
    }
}
