//! node-test-runner's own runner: the Elm program its
//! generated `main` runs, whose reporters report as elm-test's once nomi
//! builds them, and the JavaScript around the compiled program
//! (`Generate.prepareCompiledJsFile`).

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use nomi_engine::artifacts::Hash;

/// Each path under the runner's source directory, with its text.
macro_rules! sources {
    ($($path:literal),* $(,)?) => {
        [$(($path, include_str!(concat!("../runner/src/", $path)))),*]
    };
}

/// The runner's Elm modules, by path under their source directory.
const SOURCES: [(&str, &str); 14] = sources![
    "Console/Text.elm",
    "Test/Reporter/Console.elm",
    "Test/Reporter/Console/Format/Color.elm",
    "Test/Reporter/Console/Format.elm",
    "Test/Reporter/Console/Format/Monochrome.elm",
    "Test/Reporter/Highlightable.elm",
    "Test/Reporter/Json.elm",
    "Test/Reporter/JUnit.elm",
    "Test/Reporter/Reporter.elm",
    "Test/Reporter/TestResults.elm",
    "Test/Runner/JsMessage.elm",
    "Test/Runner/Node.elm",
    "Test/Runner/Node/Vendor/Console.elm",
    "Test/Runner/Node/Vendor/Diff.elm",
];

const BEFORE: &str = include_str!("../runner/before.js");
const AFTER: &str = include_str!("../runner/after.js");

/// nomi's own: a worker that runs the program as elm-test does in its own
/// process when there is one worker.
pub(crate) const DRIVER: &str = include_str!("../runner/driver.js");

/// The source directory's name, by a hash of the sources.
static NAME: LazyLock<String> = LazyLock::new(|| {
    let mut all = Vec::new();
    for (path, text) in SOURCES {
        all.extend_from_slice(path.as_bytes());
        all.push(0);
        all.extend_from_slice(text.as_bytes());
        all.push(0);
    }
    format!("runner-{}", nomi_base::hex(&Hash::of(&all).0[..8]))
});

/// The runner's source directory under `dir`, named by a hash of its
/// text and written unless it already is.
pub(crate) fn sources(dir: &Path) -> io::Result<PathBuf> {
    let target = dir.join(&*NAME);
    if target.is_dir() {
        return Ok(target);
    }
    // Written aside and moved in whole, so a run never reads it half
    // written; another run may have moved its copy in first.
    let partial = dir.join(format!("{}.{}", *NAME, std::process::id()));
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
pub(crate) fn wrap(js: &str) -> String {
    let (head, tail) = js.split_once("console.warn").unwrap_or((js, ""));
    let (before, after) = (BEFORE.trim_start(), AFTER.trim_end());
    let mut out = String::with_capacity(before.len() + js.len() + after.len() + 160);
    let _ = write!(
        out,
        "{before}\nvar Elm = (function() {{\n{head}{tail}\nreturn this.Elm;\n}}).call({{}});\nvar pipeFilename = {{ host: '127.0.0.1', port: Number(process.argv[3]) }};\n{after}"
    );
    out
}
