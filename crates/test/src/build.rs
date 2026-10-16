//! The runner a run builds: elm-test's generated project, in
//! nomi's cache rather than `elm-stuff`, its `main` over the tests the
//! types find, built through the engine and wrapped for Node.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nomi_base::{Compat, Platform, Report};
use nomi_engine::artifacts::Hash;
use nomi_engine::check::Style;
use nomi_engine::console::{Console, Personality};
use nomi_engine::javascript::{Dom, Format, Kind, Target};
use nomi_engine::make::{self, Make, Output};
use nomi_engine::packages::Resolve;
use nomi_engine::paths::Cache;
use nomi_engine::test_deps::{self, Dependencies, Solve};
use nomi_engine::{Context, Http, Vfs};
use nomi_project::outline::{AppOutline, SrcDir};
use nomi_project::{ModuleName, Name, Outline, Version};

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

/// What a build takes from where it runs.
#[derive(Debug)]
pub struct Env<'a, H> {
    /// Where the command runs; the build runs in the generated project.
    pub cx: &'a Context<'a, H>,
    pub solve: Solve,
    /// Whether elm's reports may be colored, as elm-test lets elm color
    /// them when it colors its own output.
    pub color: bool,
}

impl<H> Env<'_, H> {
    /// What a command of the engine's run in `dir` takes from it.
    pub(crate) fn at<'b>(&'b self, dir: &'b Path) -> Context<'b, H> {
        Context { dir, ..*self.cx }
    }
}

/// A step of the build, which the console's progress line names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The dependencies are solved anew.
    Solving,
    /// The tests and their runner are compiled.
    Compiling,
}

/// The generated project of `project`: its directory, with the runner's
/// sources and elm-test's `elm.json`.
fn prepare<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
    progress: &mut dyn FnMut(&mut dyn Console, Step),
) -> Result<PathBuf, Failure> {
    let dir = generated_dir(env.cx.cache, project);
    let src = dir.join("src");
    for generated in ["Test/Generated", "Test/Runner"] {
        fs::create_dir_all(src.join(generated)).map_err(|e| io(&dir, &e))?;
    }
    let runner = runner::sources(&env.cx.cache.dir().join("tests")).map_err(|e| io(&dir, &e))?;
    let Dependencies { direct, indirect } = test_deps::dependencies(
        &project.elm_json,
        &project.outline,
        &dir,
        env.cx.cache,
        env.cx.http,
        env.solve,
        &mut || progress(console, Step::Solving),
    )
    .map_err(Failure::Project)?;
    // nomi/test's tests run on Node only through a runner that has it.
    let nomi_test = Name::nomi_test();
    let effect = if direct.contains_key(&nomi_test) || indirect.contains_key(&nomi_test) {
        runner::EFFECTFUL
    } else {
        runner::PURE
    };
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
    let elm_json = dir.join("elm.json");
    write_if_changed(env.cx.vfs, &elm_json, &outline.write()).map_err(|e| io(&dir, &e))?;
    write_if_changed(env.cx.vfs, &src.join(runner::EFFECT), effect.as_bytes())
        .map_err(|e| io(&dir, &e))?;
    Ok(dir)
}

fn io(dir: &Path, e: &io::Error) -> Failure {
    Failure::Project(Report::new(
        "PROBLEM GENERATING TESTS",
        None,
        &format!(
            "I could not write the test runner to {}: {e}",
            dir.display()
        ),
        [],
    ))
}

/// `elm make` as elm-test runs it in the generated project, resolving
/// dependencies as `resolve` says.
fn elm_make(paths: Vec<PathBuf>, style: Style, output: Output, resolve: Resolve) -> Make {
    Make {
        paths,
        personality: Personality::Elm,
        style,
        kind: Ok(Kind::Dev),
        target: Target::default(),
        platform_by_main: false,
        resolve,
        ffi: None,
        output: Some(output),
        docs: None,
    }
}

/// Builds the runner of `files` in `project`, with `options`. Only the
/// build's errors show, as elm-test shows `elm make`'s standard error
/// alone.
pub fn build<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
    files: &[PathBuf],
    options: &Options,
    progress: &mut dyn FnMut(&mut dyn Console, Step),
) -> Result<Runner, Failure> {
    let found = modules(project, files).map_err(Failure::Files)?;
    let dir = prepare(env, console, project, progress)?;
    progress(console, Step::Compiling);
    let terminal = env.color && console.err_is_terminal();
    // The test modules first, for their types; then `main` over them.
    let mut quiet = Quiet::new(terminal);
    let interfaces = make::interfaces(
        &env.at(&dir),
        &mut quiet,
        &elm_make(
            files.to_vec(),
            Style::Terminal,
            Output::DevNull,
            env.solve.resolve,
        ),
    );
    let Some(interfaces) = interfaces else {
        return Err(quiet.shown(console));
    };
    let found_tests: Vec<(String, Vec<String>)> = (found.iter())
        .filter_map(|module| {
            let name = ModuleName::new(&module.name);
            let found = interfaces.iter().find(|m| m.name == name)?;
            let tests = tests(module, found.interface.get());
            (!tests.is_empty()).then(|| (module.name.clone(), tests))
        })
        .collect();
    let imports: Vec<String> = found.iter().map(|m| m.name.clone()).collect();
    let main = dir
        .join("src")
        .join(format!("{}.elm", MAIN.replace('.', "/")));
    let text = main_module(options, &imports, &found_tests);
    write_if_changed(env.cx.vfs, &main, text.as_bytes()).map_err(|e| io(&dir, &e))?;
    let js = dir.join("elmTestOutput.js");
    let mut quiet = Quiet::new(terminal);
    let mut make = elm_make(
        vec![main],
        Style::Terminal,
        Output::Js(js.clone()),
        env.solve.resolve,
    );
    // Node's kernels, in the file elm-test's templates wrap, whose
    // stand-ins for the browser's globals let tests reach the DOM's.
    make.target = Target {
        platform: Platform::Node,
        format: Format::Iife,
        dom: Dom::Polyfilled,
        ffi: None,
    };
    if !make::run(&env.at(&dir), &mut quiet, &make) {
        return Err(quiet.shown(console));
    }
    let compiled = fs::read_to_string(&js).map_err(|e| io(&dir, &e))?;
    fs::write(&js, runner::wrap(&compiled)).map_err(|e| io(&dir, &e))?;
    // Node reads the program as CommonJS, whatever a `package.json` above
    // it says.
    write_if_changed(
        env.cx.vfs,
        &dir.join("package.json"),
        br#"{"type":"commonjs"}"#,
    )
    .map_err(|e| io(&dir, &e))?;
    Ok(Runner {
        dir,
        js,
        tests: found_tests,
    })
}

/// `elm-test make`: `files` built in the generated project as `elm make
/// --output=/dev/null` builds them, with its output, or its JSON report
/// with `style`.
pub(crate) fn make<H: Http + Sync>(
    env: &Env<'_, H>,
    console: &mut dyn Console,
    project: &Project,
    files: &[PathBuf],
    style: Style,
) -> Result<(), Failure> {
    let dir = prepare(env, console, project, &mut |_, _| {})?;
    let make = elm_make(files.to_vec(), style, Output::DevNull, env.solve.resolve);
    if make::run(&env.at(&dir), console, &make) {
        Ok(())
    } else {
        Err(Failure::Shown)
    }
}

/// Where `project`'s generated project is: elm-test's
/// `elm-stuff/generated-code/elm-community/elm-test/<version>`, in nomi's
/// cache by a hash of the project's root.
pub(crate) fn generated_dir(cache: &Cache, project: &Project) -> PathBuf {
    let root = project.root.to_string_lossy();
    let hash = Hash::of(root.as_bytes());
    cache.dir().join("tests").join(nomi_base::hex(&hash.0[..8]))
}

/// Writes `bytes` to `path` unless it already holds them.
fn write_if_changed(vfs: &Vfs, path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    vfs.create(path, bytes)
}

/// A console keeping standard error and dropping standard output, as
/// elm-test runs `elm make` for its runner.
struct Quiet {
    err: String,
    terminal: bool,
}

impl Quiet {
    fn new(terminal: bool) -> Quiet {
        Quiet {
            err: String::new(),
            terminal,
        }
    }

    /// The build failed: what it printed shown on `console`.
    fn shown(self, console: &mut dyn Console) -> Failure {
        console.err(&self.err);
        Failure::Shown
    }
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

    fn err_is_terminal(&self) -> bool {
        self.terminal
    }
}
