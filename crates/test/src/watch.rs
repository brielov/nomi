//! `elm-test --watch` (`RunTests.runTests` with `watch`): a
//! run, then another each time the tests change, watched as
//! `nomi_engine::watch` watches.

use std::path::{Path, PathBuf};
use std::thread;

use nomi_engine::console::Console;
use nomi_engine::paths::{ELM_JSON, tests_dir};
use nomi_engine::watch::{Change, POLL, Snapshot, message};

use crate::command::Progress;
use crate::supervise::chalk;

/// One run: the test source directories it found.
pub(crate) type Once<'a> = dyn FnMut(&mut dyn Console, &mut Progress) -> Vec<PathBuf> + 'a;

/// Runs the tests with `once` each time they change, forever: once gives
/// the test source directories to watch.
pub(crate) fn watch(
    console: &mut dyn Console,
    progress: &mut Progress,
    root: &Path,
    color: bool,
    once: &mut Once<'_>,
) -> i32 {
    let blue = |text: &str| chalk(color, 34, text);
    // `elm.json` and `tests/` are watched whatever the project says.
    let mut snapshot = Snapshot::new(root, vec![root.join(ELM_JSON), tests_dir(root)]);
    progress.clear_console(console);
    let mut queue: Vec<Change> = Vec::new();
    loop {
        // Changes made together join the run they start.
        if !queue.is_empty() {
            let before = queue.len();
            thread::sleep(POLL);
            queue.extend(snapshot.changes());
            if queue.len() > before {
                progress.clear_line(console);
                progress.log(console, &message(&queue));
            }
        }
        queue.clear();
        let dirs = once(console, progress);
        queue.extend(snapshot.changes());
        snapshot.watch(dirs);
        if !queue.is_empty() {
            progress.clear_console(console);
            progress.log(console, &message(&queue));
            continue;
        }
        if progress.shown() {
            console.out(&format!("{}\n", blue("Watching for changes...")));
        }
        while queue.is_empty() {
            thread::sleep(POLL);
            queue.extend(snapshot.changes());
        }
        progress.clear_console(console);
        if !progress.clears() {
            let text = "Rerunning tests due to changes...";
            let bar = "=".repeat(text.len());
            console.out(&format!(
                "{}\n",
                blue(&format!("\n\n\n{text}\n{bar}\n\n\n"))
            ));
        }
        progress.log(console, &message(&queue));
    }
}
