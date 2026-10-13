//! Directory trees: copied, listed, hashed, and made at a path of a
//! given length.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Copies the directories and regular files below `from` to `to`,
/// leaving out each entry, and what's below it, whose path relative to
/// `from` `keep` refuses. Anything else, such as a symbolic link, is left
/// out too.
pub fn copy_tree(from: &Path, to: &Path, keep: impl Fn(&Path) -> bool) -> io::Result<()> {
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        fs::create_dir_all(to.join(&dir))?;
        for entry in fs::read_dir(from.join(&dir))? {
            let entry = entry?;
            let path = dir.join(entry.file_name());
            if !keep(&path) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                fs::copy(entry.path(), to.join(&path))?;
            }
        }
    }
    Ok(())
}

/// Everything below `root` but its directories, as sorted `/`-separated
/// paths relative to it.
pub fn files_below(root: &Path) -> io::Result<Vec<String>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(root).expect("a path below the root");
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The SHA-256 of `sha256sum`'s listing of `files` under `root`, a
/// `<hash>  ./<file>` line each, so one hash pins a whole tree.
pub fn tree_digest(root: &Path, files: &[String]) -> io::Result<String> {
    let mut listing = String::new();
    for file in files {
        let bytes = fs::read(root.join(file))?;
        listing.push_str(&crate::sha256_hex(&bytes));
        listing.push_str("  ./");
        listing.push_str(file);
        listing.push('\n');
    }
    Ok(crate::sha256_hex(listing.as_bytes()))
}

/// A new directory below `base` whose path is `length` bytes long: elm
/// reflows some reports around a project's path, so a test makes its
/// paths as long as the recording's were. The path is `base`'s as given,
/// so a test that wants it canonical passes a canonical `base`.
pub fn padded_dir(base: &Path, length: usize) -> io::Result<PathBuf> {
    let pad = (length.checked_sub(base.as_os_str().len() + 1))
        .filter(|&pad| pad > 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is too long to pad to {length} bytes", base.display()),
            )
        })?;
    let dir = base.join("p".repeat(pad));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path, files: &[&str]) {
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, file).unwrap();
        }
    }

    #[test]
    fn copies_keep_what_the_filter_takes() {
        let temp = tempfile::tempdir().unwrap();
        let (from, to) = (temp.path().join("from"), temp.path().join("to"));
        tree(
            &from,
            &[
                "a",
                "src/B.elm",
                "src/c/D.elm",
                "elm-stuff/x",
                "src/elm-stuff/y",
            ],
        );
        copy_tree(&from, &to, |path| !path.ends_with("elm-stuff")).unwrap();
        assert_eq!(files_below(&to).unwrap(), ["a", "src/B.elm", "src/c/D.elm"]);
        assert_eq!(
            fs::read_to_string(to.join("src/c/D.elm")).unwrap(),
            "src/c/D.elm"
        );
        let top = temp.path().join("top");
        copy_tree(&from, &top, |path| path != Path::new("a")).unwrap();
        assert!(!top.join("a").exists() && top.join("src/B.elm").exists());
    }

    #[test]
    fn digests_are_sha256sums_listing_hashed() {
        let temp = tempfile::tempdir().unwrap();
        tree(temp.path(), &["a"]);
        let files = files_below(temp.path()).unwrap();
        // `printf a > a; sha256sum ./a | sha256sum`
        let listing = "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb  ./a\n";
        assert_eq!(
            tree_digest(temp.path(), &files).unwrap(),
            crate::sha256_hex(listing.as_bytes())
        );
    }

    #[test]
    fn padded_directories_have_the_length_asked() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let length = base.as_os_str().len() + 10;
        let dir = padded_dir(&base, length).unwrap();
        assert!(dir.is_dir());
        assert_eq!(dir, base.join("ppppppppp"));
        assert!(padded_dir(&base, base.as_os_str().len() + 1).is_err());
    }
}
