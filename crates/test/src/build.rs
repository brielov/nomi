//! The runner a run builds (plan 10.3): elm-test's generated project, in
//! nomi's cache rather than `elm-stuff`, its `main` over the tests the
//! types find, built through the engine and wrapped for Node.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nomi_base::{Compat, Report};
use nomi_engine::artifacts::Hash;
use nomi_engine::check::Style;
use nomi_engine::console::{Console, Personality};
use nomi_engine::make::{self, Make, Output};
use nomi_engine::paths::Cache;
use nomi_engine::{Http, Vfs, test_deps};
use nomi_project::outline::{AppOutline, SrcDir};
use nomi_project::{ModuleName, Outline, Version};

use crate::discover::{modules, tests};
use crate::files::Project;
use crate::generate::{MAIN, Options, main_module};
use crate::runner;

/// A runner built.
#[derive(Clone, Debug)]
pub struct Runner {
    /// The generated project.
    pub dir: PathBuf,
    /// The program, ready for Node.
    pub js: PathBuf,
    /// Each module with tests, and its tests, in order.
    pub tests: Vec<(String, Vec<String>)>,
}

/// Why there is no runner.
#[derive(Debug)]
pub enum Failure {
    /// elm-test's message about the test files.
    Files(String),
    /// What stops the generated project being built.
    Project(Report),
    /// The build failed, and its reports are shown.
    Shown,
}

/// Builds the runner of `files` in `project`, with `options`. Only the build's errors show, as elm-test
/// shows `elm make`'s standard error alone.
#[allow(clippy::too_many_arguments)]
pub fn build(
    vfs: &Vfs,
    console: &mut dyn Console,
    cache: &Cache,
    http: &(impl Http + Sync),
    project: &Project,
    files: &[PathBuf],
    options: &Options,
) -> Result<Runner, Failure> {
    let found = modules(project, files).map_err(Failure::Files)?;
    let tests_dir = cache.dir().join("tests");
    let dir = tests_dir.join(hex(project.root.to_string_lossy().as_bytes()));
    let src = dir.join("src");
    let io = |e: io::Error| {
        Failure::Project(Report::new(
            "PROBLEM GENERATING TESTS",
            None,
            &format!(
                "I could not write the test runner to {}: {e}",
                dir.display()
            ),
            [],
        ))
    };
    fs::create_dir_all(src.join("Test/Generated")).map_err(io)?;
    let runner = runner::sources(&tests_dir).map_err(io)?;
    let (direct, indirect) =
        test_deps::dependencies(&project.elm_json, &project.outline, &dir, cache, http)
            .map_err(Failure::Project)?;
    let mut source_dirs = vec![src.clone(), runner];
    for source_dir in &project.source_dirs {
        if !source_dirs.contains(source_dir) {
            source_dirs.push(source_dir.clone());
        }
    }
    let outline = Outline::App(AppOutline {
        elm: Version::compiler(Compat::V0_19_3),
        source_dirs: (source_dirs.iter())
            .map(|d| SrcDir::new(&d.to_string_lossy()))
            .collect(),
        direct,
        indirect,
        test_direct: Default::default(),
        test_indirect: Default::default(),
    });
    write_if_changed(vfs, &dir.join("elm.json"), &outline.write()).map_err(io)?;
    // The test modules first, for their types; then `main` over them.
    let make = |paths: Vec<PathBuf>, output| Make {
        paths,
        personality: Personality::Elm,
        style: Style::Terminal,
        debug: false,
        optimize: false,
        output: Some(output),
        docs: None,
    };
    let mut quiet = Quiet::default();
    let interfaces = make::interfaces(
        vfs,
        &mut quiet,
        &dir,
        cache,
        http,
        &make(files.to_vec(), Output::DevNull),
    );
    let Some(interfaces) = interfaces else {
        console.err(&quiet.err);
        return Err(Failure::Shown);
    };
    let found_tests: Vec<(String, Vec<String>)> = (found.iter())
        .filter_map(|module| {
            let name = ModuleName::new(&module.name);
            let (_, interface) = interfaces.iter().find(|(n, _)| *n == name)?;
            let tests = tests(module, interface.get());
            (!tests.is_empty()).then(|| (module.name.clone(), tests))
        })
        .collect();
    let imports: Vec<String> = found.iter().map(|m| m.name.clone()).collect();
    let main = src.join(format!("{}.elm", MAIN.replace('.', "/")));
    let text = main_module(options, &imports, &found_tests);
    write_if_changed(vfs, &main, text.as_bytes()).map_err(io)?;
    let js = dir.join("elmTestOutput.js");
    let mut quiet = Quiet::default();
    let made = make::run(
        vfs,
        &mut quiet,
        &dir,
        cache,
        http,
        &make(vec![main], Output::Js(js.clone())),
    );
    if !made {
        console.err(&quiet.err);
        return Err(Failure::Shown);
    }
    let compiled = fs::read_to_string(&js).map_err(io)?;
    fs::write(&js, runner::wrap(&compiled)).map_err(io)?;
    // Node reads the program as CommonJS, whatever a `package.json` above
    // it says.
    write_if_changed(vfs, &dir.join("package.json"), br#"{"type":"commonjs"}"#).map_err(io)?;
    Ok(Runner {
        dir,
        js,
        tests: found_tests,
    })
}

/// A short hex name for `bytes`.
fn hex(bytes: &[u8]) -> String {
    (Hash::of(bytes).0.iter().take(8))
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Writes `bytes` to `path` unless it already holds them.
fn write_if_changed(vfs: &Vfs, path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    vfs.create(path, bytes)
}

/// A console keeping standard error and dropping standard output.
#[derive(Default)]
struct Quiet {
    err: String,
}

impl Console for Quiet {
    fn out(&mut self, _: &str) {}

    fn err(&mut self, text: &str) {
        self.err.push_str(text);
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        Ok(None)
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        Ok(Vec::new())
    }
}
