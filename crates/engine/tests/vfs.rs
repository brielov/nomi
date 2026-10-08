//! The VFS against a real directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nomi_engine::Vfs;

const MODULE: &str = "module A exposing (x)\n\n\nx =\n    1\n";

fn touch(root: &Path, path: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, MODULE).unwrap();
}

/// `paths` relative to `root`, as `/`-separated strings.
fn relative(root: &Path, paths: Vec<PathBuf>) -> Vec<String> {
    paths
        .iter()
        .map(|p| {
            let p = p.strip_prefix(root).unwrap();
            p.components()
                .map(|c| c.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

#[test]
fn discovery_finds_elm_files_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for path in [
        "b/E.elm",
        "a/F.elm",
        "src/Z/G.elm",
        "H.elm",
        "c.elm",
        ".hidden/D.elm",
        "notes.txt",
        "elm-stuff/A.elm",
        "node_modules/B.elm",
        ".git/C.elm",
        "src/elm-stuff/I.elm",
    ] {
        touch(root, path);
    }
    let files = Vfs::new().discover(root).unwrap();
    assert_eq!(
        relative(root, files),
        [
            ".hidden/D.elm",
            "H.elm",
            "a/F.elm",
            "b/E.elm",
            "c.elm",
            "src/Z/G.elm"
        ]
    );
}

#[test]
fn a_file_is_itself_whatever_its_extension() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "notes.txt");
    let file = dir.path().join("notes.txt");
    assert_eq!(Vfs::new().discover(&file).unwrap(), [file]);
}

#[test]
fn a_skipped_root_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "elm-stuff/A.elm");
    let files = Vfs::new().discover(&dir.path().join("elm-stuff")).unwrap();
    assert!(files.is_empty());
}

#[test]
fn a_missing_root_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = Vfs::new().discover(&dir.path().join("gone")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn discovery_follows_symbolic_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a/F.elm");
    std::os::unix::fs::symlink(root.join("a"), root.join("z")).unwrap();
    let files = Vfs::new().discover(root).unwrap();
    assert_eq!(relative(root, files), ["a/F.elm", "z/F.elm"]);
}

#[test]
fn reading_takes_utf8_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, MODULE).unwrap();
    assert_eq!(Vfs::new().read(&path).unwrap(), MODULE);
    fs::write(&path, b"x = \xff\n").unwrap();
    let error = Vfs::new().read(&path).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn writing_replaces_the_file_and_leaves_nothing_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, "old").unwrap();
    Vfs::new().write(&path, MODULE).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), MODULE);
    let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn writing_a_missing_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = Vfs::new()
        .write(&dir.path().join("A.elm"), MODULE)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn writing_keeps_permissions_and_links() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let link = dir.path().join("B.elm");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    Vfs::new().write(&link, MODULE).unwrap();
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), MODULE);
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
}
