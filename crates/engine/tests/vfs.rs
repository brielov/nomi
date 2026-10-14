//! The VFS against a real directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nomi_engine::{Vfs, discover};

const MODULE: &str = "module A exposing (x)\n\n\nx =\n    1\n";

fn touch(root: &Path, path: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, MODULE).unwrap();
}

/// `paths` relative to `root`, as `/`-separated strings.
fn relative(root: &Path, paths: &[PathBuf]) -> Vec<String> {
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
    let files = discover(root).unwrap();
    assert_eq!(
        relative(root, &files),
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
    assert_eq!(discover(&file).unwrap(), [file]);
}

#[test]
fn a_skipped_root_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "elm-stuff/A.elm");
    let files = discover(&dir.path().join("elm-stuff")).unwrap();
    assert!(files.is_empty());
}

#[test]
fn a_missing_root_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = discover(&dir.path().join("gone")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn discovery_follows_symbolic_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a/F.elm");
    std::os::unix::fs::symlink(root.join("a"), root.join("z")).unwrap();
    let files = discover(root).unwrap();
    assert_eq!(relative(root, &files), ["a/F.elm", "z/F.elm"]);
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

#[test]
fn creating_writes_a_missing_file_or_replaces_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    Vfs::new().create(&path, "old").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "old");
    Vfs::new().create(&path, MODULE).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), MODULE);
    let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
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

#[cfg(unix)]
#[test]
fn a_file_has_one_id_through_any_link() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a/F.elm");
    std::os::unix::fs::symlink(root.join("a"), root.join("z")).unwrap();
    std::os::unix::fs::symlink(root.join("a/F.elm"), root.join("G.elm")).unwrap();
    let vfs = Vfs::new();
    let id = vfs.file_id(&root.join("a/F.elm")).unwrap();
    for alias in ["z/F.elm", "G.elm", "a/../z/./F.elm"] {
        assert_eq!(vfs.file_id(&root.join(alias)).unwrap(), id, "{alias}");
    }
    assert_ne!(vfs.file_id(&root.join("a/G.elm")).unwrap(), id);
    assert_eq!(
        *vfs.path(id),
        fs::canonicalize(root.join("a/F.elm")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn a_missing_file_is_named_through_its_nearest_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    std::os::unix::fs::symlink(root.join("a"), root.join("z")).unwrap();
    let vfs = Vfs::new();
    let id = vfs.file_id(&root.join("a/new/New.elm")).unwrap();
    assert_eq!(vfs.file_id(&root.join("z/new/./New.elm")).unwrap(), id);
    assert_eq!(vfs.file_id(&root.join("z/new/x/../New.elm")).unwrap(), id);
    // Climbing out of a missing directory resolves links again.
    let through = root.join("gone/deeper/../../z/new/New.elm");
    assert_eq!(vfs.file_id(&through).unwrap(), id);
    vfs.set_overlay(&through, "module New exposing (..)\n")
        .unwrap();
    let path = root.join("a/new/New.elm");
    assert_eq!(vfs.read(&path).unwrap(), "module New exposing (..)\n");
    vfs.remove_overlay(&path).unwrap();
    assert!(vfs.read(&through).is_err());
}

#[test]
fn overlays_hide_the_disk_until_removed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, MODULE).unwrap();
    let vfs = Vfs::new();
    let start = vfs.revision();
    let set = vfs.set_overlay(&path, "module A exposing (y)\n").unwrap();
    assert!(set > start);
    assert_eq!(vfs.read(&path).unwrap(), "module A exposing (y)\n");
    assert_eq!(vfs.read_bytes(&path).unwrap(), b"module A exposing (y)\n");
    let unsaved = dir.path().join("B.elm");
    vfs.set_overlay(&unsaved, "module B exposing (..)\n")
        .unwrap();
    assert_eq!(vfs.read(&unsaved).unwrap(), "module B exposing (..)\n");
    assert!(!vfs.is_file(&unsaved));
    vfs.remove_overlay(&path).unwrap();
    assert_eq!(vfs.read(&path).unwrap(), MODULE);
}

#[test]
fn a_snapshot_keeps_its_overlays_and_knows_when_it_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, MODULE).unwrap();
    let vfs = Vfs::new();
    vfs.set_overlay(&path, "old").unwrap();
    let snapshot = vfs.snapshot();
    assert!(!snapshot.is_stale());
    vfs.set_overlay(&path, "new").unwrap();
    assert!(snapshot.is_stale());
    let id = snapshot.file_id(&path).unwrap();
    assert_eq!(&*snapshot.text(id).unwrap(), "old");
    assert_eq!(&*vfs.snapshot().text(id).unwrap(), "new");
    assert!(snapshot.revision() < vfs.revision());
}

#[test]
fn writing_makes_a_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, MODULE).unwrap();
    let vfs = Vfs::new();
    let snapshot = vfs.snapshot();
    vfs.write(&path, "x").unwrap();
    assert!(snapshot.is_stale());
    let before = vfs.revision();
    assert!(vfs.disk_changed() > before);
}
