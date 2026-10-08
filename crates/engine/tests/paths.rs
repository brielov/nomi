//! Finding a project's root on a real directory.

use std::fs;

use nomi_engine::Vfs;
use nomi_engine::paths::Root;

#[test]
fn roots_are_the_nearest_directory_with_an_elm_json() {
    let temp = tempfile::tempdir().unwrap();
    let top = temp.path().canonicalize().unwrap();
    let app = top.join("app");
    let deep = app.join("src/Page");
    fs::create_dir_all(&deep).unwrap();
    // A directory named `elm.json` doesn't make a project.
    fs::create_dir_all(app.join("src/elm.json")).unwrap();
    let vfs = Vfs::new();
    let outside = Root::find(&vfs, &deep).filter(|r| r.dir().starts_with(&top));
    assert_eq!(outside, None);
    fs::write(app.join("elm.json"), "{}").unwrap();
    assert_eq!(Root::find(&vfs, &deep), Some(Root::new(app.clone())));
    assert_eq!(Root::find(&vfs, &app), Some(Root::new(app.clone())));
    fs::write(top.join("elm.json"), "{}").unwrap();
    assert_eq!(
        Root::find(&vfs, &deep).unwrap().elm_json(),
        app.join("elm.json")
    );
    assert_eq!(Root::find(&vfs, &top), Some(Root::new(top.clone())));
}

#[cfg(unix)]
#[test]
fn a_link_to_an_elm_json_makes_a_root() {
    let temp = tempfile::tempdir().unwrap();
    let top = temp.path().canonicalize().unwrap();
    fs::write(top.join("real.json"), "{}").unwrap();
    let app = top.join("app");
    fs::create_dir(&app).unwrap();
    std::os::unix::fs::symlink(top.join("real.json"), app.join("elm.json")).unwrap();
    assert_eq!(Root::find(&Vfs::new(), &app), Some(Root::new(app)));
}
