//! The per-user artifact cache (architecture §6): what building a module
//! made, filed under a key that hashes everything the build read, so any
//! project and any checkout reuse it. It trims itself as Go's `GOCACHE`
//! does: a read marks an entry used, and once a day the entries unused
//! for five days go.
//!
//! An entry is the key, the payload's length and SHA-256, then the payload.
//! One that doesn't check out, however it got that way, is a miss; the
//! build makes it again and replaces it. Entries are written to a
//! temporary file and renamed into place, so readers never see one half
//! written, and builds running at once can share the cache.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::paths;

/// Entries in a newer layout go in a new directory, so an older nomi's
/// never reads as one.
const LAYOUT: &str = "v1";
const MAGIC: &[u8; 8] = b"NOMIART1";
const HEADER: usize = MAGIC.len() + 32 + 8 + 32;
/// The largest payload read back; a larger file is a miss.
const MAX_PAYLOAD: u64 = 1 << 30;

/// How old an entry's time may get before a read refreshes it (Go's
/// `mtimeInterval`), so most reads don't write.
const REFRESH: Duration = Duration::from_secs(60 * 60);
/// How long between trims (`trimInterval`).
const TRIM_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long an entry stays unused before a trim removes it (`trimLimit`).
const UNUSED_FOR: Duration = Duration::from_secs(5 * 24 * 60 * 60);

/// A SHA-256 hash: of an artifact's inputs, or of an interface.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Hash {
        Hash(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({self})")
    }
}

/// What an artifact was made from, hashed: the source, the build of nomi,
/// `Compat`, the interface hashes of resolved imports, the package. Each
/// part is named and its length written, so no two lists of parts hash
/// alike by running into each other.
#[derive(Clone, Debug)]
pub struct Key {
    hasher: Sha256,
}

impl Key {
    /// A key for artifacts of `kind`, made by this build of nomi.
    pub fn new(kind: &str) -> Key {
        Key::made_by(kind, &build_id().0)
    }

    /// A key for artifacts of `kind`, made by the build `toolchain` names.
    fn made_by(kind: &str, toolchain: &[u8]) -> Key {
        Key {
            hasher: Sha256::new(),
        }
        .part("kind", kind.as_bytes())
        .part("toolchain", toolchain)
    }

    pub fn part(mut self, name: &str, bytes: &[u8]) -> Key {
        for bytes in [name.as_bytes(), bytes] {
            self.hasher.update((bytes.len() as u64).to_le_bytes());
            self.hasher.update(bytes);
        }
        self
    }

    pub fn hash(self, name: &str, hash: Hash) -> Key {
        self.part(name, &hash.0)
    }

    pub fn finish(self) -> Hash {
        Hash(self.hasher.finalize().into())
    }
}

/// What names this build of nomi: its executable, by canonical path, file
/// identity, size and modification time, so every rebuild is a different
/// toolchain, though every development build has one version number.
/// Hashing the executable's bytes, as Go's build ID does, would cost every
/// run tens of milliseconds; a binary that changes keeps none of these the
/// same. Made once per process, when the cache is first used. If the
/// executable can't be found, this run's artifacts are its own.
fn build_id() -> Hash {
    static ID: OnceLock<Hash> = OnceLock::new();
    *ID.get_or_init(|| {
        let identity = std::env::current_exe()
            .and_then(fs::canonicalize)
            .and_then(|exe| Ok((fs::metadata(&exe)?, exe)))
            .map(|(meta, exe)| executable_identity(&exe, &meta));
        identity.unwrap_or_else(|_| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let run = format!("unreadable {} {}", std::process::id(), now.as_nanos());
            Hash::of(run.as_bytes())
        })
    })
}

/// [`build_id`] of the executable at `exe`.
fn executable_identity(exe: &std::path::Path, meta: &fs::Metadata) -> Hash {
    let modified = (meta.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let file = (0u64, 0u64);
    let identity = format!(
        "{} {} {} {} {}",
        exe.display(),
        file.0,
        file.1,
        meta.len(),
        modified.as_nanos()
    );
    Hash::of(identity.as_bytes())
}

/// The artifact cache in the user's cache directory.
#[derive(Clone, Debug)]
pub struct Artifacts {
    dir: PathBuf,
}

/// What a trim did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trimmed {
    pub removed: usize,
    pub kept: usize,
}

impl Artifacts {
    pub fn new(cache: &paths::Cache) -> Artifacts {
        Artifacts {
            dir: cache.artifacts().join(LAYOUT),
        }
    }

    fn path(&self, key: Hash) -> PathBuf {
        let hex = key.to_string();
        self.dir.join(&hex[..2]).join(hex)
    }

    /// The payload filed under `key`, if there is a sound one; it is then
    /// marked used.
    pub fn get(&self, key: Hash) -> Option<Vec<u8>> {
        self.get_hashed(key).map(|(payload, _)| payload)
    }

    /// [`Artifacts::get`], with the payload's hash, which the entry holds.
    pub fn get_hashed(&self, key: Hash) -> Option<(Vec<u8>, Hash)> {
        self.get_at(key, SystemTime::now())
    }

    fn get_at(&self, key: Hash, now: SystemTime) -> Option<(Vec<u8>, Hash)> {
        let path = self.path(key);
        let mut file = fs::File::open(&path).ok()?;
        let meta = file.metadata().ok()?;
        // Read whole at the size the file has, which a sound entry's
        // header agrees with.
        let length = meta.len().checked_sub(HEADER as u64)?;
        if length > MAX_PAYLOAD {
            return None;
        }
        let mut header = [0; HEADER];
        file.read_exact(&mut header).ok()?;
        let mut payload = vec![0; usize::try_from(length).ok()?];
        file.read_exact(&mut payload).ok()?;
        let hash = open(&header, &payload, key)?;
        let used = meta.modified().ok()?;
        if now.duration_since(used).is_ok_and(|age| age >= REFRESH) {
            // Best effort: the entry was sound, whether or not this lands.
            let _ = fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_modified(now));
        }
        Some((payload, hash))
    }

    /// Files `payload` under `key`, replacing any entry there.
    pub fn put(&self, key: Hash, payload: &[u8]) -> io::Result<()> {
        let path = self.path(key);
        let dir = path.parent().expect("an entry is in a directory");
        fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(MAGIC)?;
        file.write_all(&key.0)?;
        file.write_all(&(payload.len() as u64).to_le_bytes())?;
        file.write_all(&Hash::of(payload).0)?;
        file.write_all(payload)?;
        file.persist(&path).map_err(|e| e.error)?;
        Ok(())
    }

    /// Removes the entries unused for five days, unless a trim ran in the
    /// last day.
    pub fn trim(&self) -> io::Result<Trimmed> {
        self.trim_at(SystemTime::now())
    }

    fn trim_at(&self, now: SystemTime) -> io::Result<Trimmed> {
        let stamp = self.dir.join("trim.txt");
        let last = fs::read_to_string(&stamp)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
        if last.is_some_and(|last| now.duration_since(last).is_ok_and(|age| age < TRIM_EVERY)) {
            return Ok(Trimmed::default());
        }
        let mut trimmed = Trimmed::default();
        let cutoff = now.checked_sub(UNUSED_FOR).unwrap_or(UNIX_EPOCH);
        let dirs = match fs::read_dir(&self.dir) {
            Ok(dirs) => dirs,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(trimmed),
            Err(e) => return Err(e),
        };
        for dir in dirs {
            let dir = dir?;
            if !dir.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(dir.path())? {
                let entry = entry?;
                let used = entry.metadata()?.modified()?;
                if used < cutoff {
                    match fs::remove_file(entry.path()) {
                        Ok(()) => trimmed.removed += 1,
                        // Another trim got there first.
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e),
                    }
                } else {
                    trimmed.kept += 1;
                }
            }
        }
        let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let mut file = tempfile::NamedTempFile::new_in(&self.dir)?;
        writeln!(file, "{secs}")?;
        file.persist(&stamp).map_err(|e| e.error)?;
        Ok(trimmed)
    }
}

/// The hash of `payload`, if with `header` it is a sound entry filed
/// under `key`.
fn open(header: &[u8; HEADER], payload: &[u8], key: Hash) -> Option<Hash> {
    let (magic, rest) = header.split_at(MAGIC.len());
    let (filed, rest) = rest.split_at(32);
    let (length, digest) = rest.split_at(8);
    let length = u64::from_le_bytes(length.try_into().expect("eight bytes"));
    if magic != MAGIC || filed != key.0 || length != payload.len() as u64 {
        return None;
    }
    let hash = Hash::of(payload);
    (digest == hash.0).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifacts() -> (tempfile::TempDir, Artifacts) {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = Artifacts::new(&paths::Cache::new(dir.path().to_owned()));
        (dir, artifacts)
    }

    #[test]
    fn keys_hash_their_parts_and_where_each_ends() {
        let key = |parts: &[(&str, &str)]| {
            parts
                .iter()
                .fold(Key::new("test"), |k, (n, b)| k.part(n, b.as_bytes()))
                .finish()
        };
        assert_eq!(key(&[("a", "bc")]), key(&[("a", "bc")]));
        assert_ne!(key(&[("a", "bc")]), key(&[("ab", "c")]));
        assert_ne!(key(&[("a", "b"), ("c", "")]), key(&[("a", "bc")]));
        assert_ne!(Key::new("x").finish(), Key::new("y").finish());
    }

    #[test]
    fn keys_belong_to_the_build_that_made_them() {
        assert_eq!(Key::new("test").finish(), Key::new("test").finish());
        let other = Key::made_by("test", b"another build").finish();
        assert_ne!(Key::new("test").finish(), other);
        assert_eq!(
            Key::made_by("test", b"one build").finish(),
            Key::made_by("test", b"one build").finish()
        );
        let exe = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let identity = executable_identity(&exe, &fs::metadata(&exe).unwrap());
        assert_eq!(build_id(), identity);
        // A rebuilt binary, here a copy, is another toolchain.
        let temp = tempfile::tempdir().unwrap();
        let copy = temp.path().join("nomi");
        fs::copy(&exe, &copy).unwrap();
        assert_ne!(
            executable_identity(&copy, &fs::metadata(&copy).unwrap()),
            identity
        );
    }

    #[test]
    fn entries_read_back_until_they_are_damaged() {
        let (_dir, artifacts) = artifacts();
        let key = Key::new("test").finish();
        assert_eq!(artifacts.get(key), None);
        artifacts.put(key, b"payload").unwrap();
        assert_eq!(artifacts.get(key).as_deref(), Some(&b"payload"[..]));
        artifacts.put(key, b"other").unwrap();
        assert_eq!(artifacts.get(key).as_deref(), Some(&b"other"[..]));

        let path = artifacts.path(key);
        let sound = fs::read(&path).unwrap();
        for at in [0, 8, 40, 48, sound.len() - 1] {
            let mut bytes = sound.clone();
            bytes[at] ^= 1;
            fs::write(&path, bytes).unwrap();
            assert_eq!(artifacts.get(key), None, "byte {at}");
        }
        fs::write(&path, &sound[..sound.len() - 1]).unwrap();
        assert_eq!(artifacts.get(key), None);
        // Filed under another key.
        let other = Key::new("other").finish();
        fs::create_dir_all(artifacts.path(other).parent().unwrap()).unwrap();
        fs::write(artifacts.path(other), &sound).unwrap();
        assert_eq!(artifacts.get(other), None);
    }

    #[test]
    fn trimming_removes_what_went_unused() {
        let (_dir, artifacts) = artifacts();
        let day = Duration::from_secs(24 * 60 * 60);
        let start = SystemTime::now();
        let old = Key::new("old").finish();
        let used = Key::new("used").finish();
        let new = Key::new("new").finish();
        for key in [old, used, new] {
            artifacts.put(key, b"x").unwrap();
            let file = fs::File::options().write(true).open(artifacts.path(key));
            file.unwrap().set_modified(start).unwrap();
        }
        assert!(artifacts.get_at(used, start + 3 * day).is_some());
        let now = start + 6 * day;
        let file = fs::File::options().write(true).open(artifacts.path(new));
        file.unwrap().set_modified(now).unwrap();
        let trimmed = artifacts.trim_at(now).unwrap();
        assert_eq!(
            trimmed,
            Trimmed {
                removed: 1,
                kept: 2
            }
        );
        assert!(!artifacts.path(old).exists());
        assert!(artifacts.get(used).is_some() && artifacts.get(new).is_some());
        // Not again within a day.
        assert_eq!(
            artifacts.trim_at(now + 20 * day / 24).unwrap(),
            Trimmed::default()
        );
        assert_eq!(artifacts.trim_at(now + day).unwrap().kept, 2);
    }

    #[test]
    fn a_read_marks_an_entry_used_at_most_hourly() {
        let (_dir, artifacts) = artifacts();
        let key = Key::new("test").finish();
        artifacts.put(key, b"x").unwrap();
        let start = SystemTime::now();
        let file = fs::File::options().write(true).open(artifacts.path(key));
        file.unwrap().set_modified(start).unwrap();
        let used = || {
            fs::metadata(artifacts.path(key))
                .unwrap()
                .modified()
                .unwrap()
        };
        artifacts.get_at(key, start + REFRESH / 2).unwrap();
        assert_eq!(used(), start);
        artifacts.get_at(key, start + REFRESH).unwrap();
        assert_eq!(used(), start + REFRESH);
    }
}
