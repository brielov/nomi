//! The registry's files, read and written whole: a missing file reads as
//! none, and a file is written in one step, so a run interrupted or racing
//! another leaves no half-written file behind.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// The bytes of the file at `path`, or `None` if there is none.
pub(crate) fn read_if_exists(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes `bytes` at `path` in one step, creating its directory.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back_and_nothing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b");
        assert_eq!(read_if_exists(&path).unwrap(), None);
        write_atomically(&path, b"one").unwrap();
        write_atomically(&path, b"two").unwrap();
        assert_eq!(read_if_exists(&path).unwrap(), Some(b"two".to_vec()));
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}
