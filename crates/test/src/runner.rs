//! node-test-runner's own runner: the Elm program its
//! generated `main` runs, whose reporters report as elm-test's once nomi
//! builds them, and the JavaScript around the compiled program
//! (`Generate.prepareCompiledJsFile`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nomi_engine::artifacts::Hash;

/// The runner's Elm modules, by path under their source directory.
const SOURCES: [(&str, &str); 14] = [
    (
        "Console/Text.elm",
        include_str!("../runner/src/Console/Text.elm"),
    ),
    (
        "Test/Reporter/Console.elm",
        include_str!("../runner/src/Test/Reporter/Console.elm"),
    ),
    (
        "Test/Reporter/Console/Format/Color.elm",
        include_str!("../runner/src/Test/Reporter/Console/Format/Color.elm"),
    ),
    (
        "Test/Reporter/Console/Format.elm",
        include_str!("../runner/src/Test/Reporter/Console/Format.elm"),
    ),
    (
        "Test/Reporter/Console/Format/Monochrome.elm",
        include_str!("../runner/src/Test/Reporter/Console/Format/Monochrome.elm"),
    ),
    (
        "Test/Reporter/Highlightable.elm",
        include_str!("../runner/src/Test/Reporter/Highlightable.elm"),
    ),
    (
        "Test/Reporter/Json.elm",
        include_str!("../runner/src/Test/Reporter/Json.elm"),
    ),
    (
        "Test/Reporter/JUnit.elm",
        include_str!("../runner/src/Test/Reporter/JUnit.elm"),
    ),
    (
        "Test/Reporter/Reporter.elm",
        include_str!("../runner/src/Test/Reporter/Reporter.elm"),
    ),
    (
        "Test/Reporter/TestResults.elm",
        include_str!("../runner/src/Test/Reporter/TestResults.elm"),
    ),
    (
        "Test/Runner/JsMessage.elm",
        include_str!("../runner/src/Test/Runner/JsMessage.elm"),
    ),
    (
        "Test/Runner/Node.elm",
        include_str!("../runner/src/Test/Runner/Node.elm"),
    ),
    (
        "Test/Runner/Node/Vendor/Console.elm",
        include_str!("../runner/src/Test/Runner/Node/Vendor/Console.elm"),
    ),
    (
        "Test/Runner/Node/Vendor/Diff.elm",
        include_str!("../runner/src/Test/Runner/Node/Vendor/Diff.elm"),
    ),
];

const BEFORE: &str = include_str!("../runner/before.js");
const AFTER: &str = include_str!("../runner/after.js");

/// nomi's own: a worker that runs the program as elm-test does in its own
/// process when there is one worker.
pub const DRIVER: &str = include_str!("../runner/driver.js");

/// The runner's source directory under `dir`, named by a hash of its
/// text and written unless it already is.
pub fn sources(dir: &Path) -> io::Result<PathBuf> {
    let mut all = Vec::new();
    for (path, text) in SOURCES {
        all.extend_from_slice(path.as_bytes());
        all.push(0);
        all.extend_from_slice(text.as_bytes());
        all.push(0);
    }
    let hex: String = (Hash::of(&all).0.iter().take(8))
        .map(|b| format!("{b:02x}"))
        .collect();
    let target = dir.join(format!("runner-{hex}"));
    if target.is_dir() {
        return Ok(target);
    }
    // Written aside and moved in whole, so a run never reads it half
    // written; another run may have moved its copy in first.
    let partial = dir.join(format!("runner-{hex}.{}", std::process::id()));
    for (path, text) in SOURCES {
        let path = partial.join(path);
        fs::create_dir_all(path.parent().unwrap_or(&partial))?;
        fs::write(path, text)?;
    }
    if let Err(e) = fs::rename(&partial, &target) {
        let _ = fs::remove_dir_all(&partial);
        if !target.is_dir() {
            return Err(e);
        }
    }
    Ok(target)
}

/// The compiled program `js` as elm-test runs it: between its templates,
/// without the development mode's warning. elm-test also tags `Test`'s
/// constructors for its `check`, which nomi's `main` doesn't call. elm-test
/// writes its pipe's path in; nomi's supervisor listens on a loopback
/// port, which `net.createConnection` takes as options, given to each
/// worker after its index.
pub fn wrap(js: &str) -> String {
    let js = js.replacen("console.warn", "", 1);
    let pipe = "{ host: '127.0.0.1', port: Number(process.argv[3]) }";
    format!(
        "\n{BEFORE}\nvar Elm = (function() {{\n{js}\nreturn this.Elm;\n}}).call({{}});\nvar pipeFilename = {pipe};\n{AFTER}\n  "
    )
    .trim()
    .to_owned()
}
