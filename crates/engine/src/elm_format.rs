//! elm-format 0.8.8's command line, for nomi invoked as `elm-format`: what
//! each combination of options does, and every message it prints, ported
//! from upstream's `ElmFormat/Cli.hs`, `ElmFormat/Messages.hs` and
//! `CommandLine/TransformFiles.hs`. The options arrive parsed; the CLI
//! emulates upstream's parser.

use std::io;
use std::path::{Path, PathBuf};

use nomi_base::ElmPosition;

use crate::console::Console;
use crate::format::{decode, format_text};
use crate::{Pool, Vfs, paths};

/// The `--elm-version` values upstream accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElmVersion {
    V0_17,
    V0_18,
    V0_19,
}

/// elm-format's options, as its parser leaves them.
#[derive(Debug, Default)]
pub struct Options {
    /// The INPUT paths, in the order upstream's parser collects them:
    /// the reverse of the command line's.
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    pub yes: bool,
    pub validate: bool,
    pub stdin: bool,
    pub elm_version: Option<ElmVersion>,
}

/// How a run ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Success,
    Failure,
    /// There was nothing to do; upstream prints its help and succeeds.
    Usage,
}

/// Where upstream reads from and writes to, once its options agree.
enum Task {
    StdinToStdout,
    StdinToFile(PathBuf),
    FileToFile(PathBuf, PathBuf),
    FilesInPlace(Vec<PathBuf>),
    ValidateStdin,
    ValidateFiles(Vec<PathBuf>),
}

/// What became of one module formatted in place.
enum Outcome {
    Unchanged,
    Formatted,
    Invalid(ElmPosition),
    Failed(io::Error),
}

/// Runs elm-format with `options`, as upstream's `main'` does.
pub fn run(vfs: &Vfs, options: &Options, console: &mut dyn Console) -> Exit {
    let files = match resolve(vfs, &options.inputs) {
        Ok(files) => files,
        Err(problems) => {
            let mut text = String::from(
                "There was a problem reading one or more of the specified INPUT paths:\n\n",
            );
            for problem in problems {
                text.push_str(&format!("    {problem}\n"));
            }
            text.push_str("\nPlease check the given paths.\n\n");
            console.err(&text);
            return Exit::Failure;
        }
    };
    let task = match task(options, files) {
        Ok(Some(task)) => task,
        Ok(None) => return Exit::Usage,
        Err(message) => {
            console.err(message);
            return Exit::Failure;
        }
    };
    let version = options.elm_version.unwrap_or_else(|| detect_version(vfs));
    if version != ElmVersion::V0_19 {
        console.err(unsupported(version, options.elm_version.is_some()));
        return Exit::Failure;
    }
    let succeeded = match task {
        Task::StdinToStdout => stdin_to(console, |console, text| {
            console.out(&text);
            true
        }),
        Task::StdinToFile(output) => {
            stdin_to(console, |console, text| write(vfs, &output, &text, console))
        }
        Task::FileToFile(input, output) => {
            console.out(&processing(&input));
            match read(vfs, &input).map(|text| text.and_then(|text| format_text(&text))) {
                Ok(Ok(text)) => write(vfs, &output, &text, console),
                Ok(Err(at)) => {
                    console.out(&unable_to_parse(&input.display().to_string(), at));
                    false
                }
                Err(e) => {
                    console.err(&crashed(&input, &e));
                    false
                }
            }
        }
        Task::FilesInPlace(files) => {
            if !options.yes {
                match approve(&files, console) {
                    Ok(true) => {}
                    Ok(false) => return Exit::Success,
                    Err(message) => {
                        console.err(&message);
                        return Exit::Failure;
                    }
                }
            }
            in_place(vfs, files, console)
        }
        Task::ValidateStdin => match console.read_all() {
            Ok(bytes) => {
                let entry = validation_entry("<STDIN>", is_formatted(bytes));
                let succeeded = entry.is_none();
                console.out(&json_list(entry.as_slice()));
                succeeded
            }
            Err(e) => {
                console.err(&format!("elm-format: <stdin>: {e}\n"));
                false
            }
        },
        Task::ValidateFiles(files) => {
            let results = Pool::global().map(files, |file| {
                let formatted = vfs.read_bytes(&file).map(is_formatted);
                (file, formatted)
            });
            let mut succeeded = true;
            let mut entries = Vec::new();
            for (file, formatted) in results {
                match formatted {
                    Ok(formatted) => {
                        entries.extend(validation_entry(&file.display().to_string(), formatted));
                    }
                    Err(e) => {
                        console.err(&crashed(&file, &e));
                        succeeded = false;
                    }
                }
            }
            succeeded &= entries.is_empty();
            console.out(&json_list(&entries));
            succeeded
        }
    };
    if succeeded {
        Exit::Success
    } else {
        Exit::Failure
    }
}

/// The modules `inputs` name, or a line for each one that names none.
fn resolve(vfs: &Vfs, inputs: &[PathBuf]) -> Result<Vec<PathBuf>, Vec<String>> {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    for input in inputs {
        let shown = input.display();
        match vfs.discover(input) {
            Ok(found) if found.is_empty() => problems.push(format!(
                "{shown}: Directory does not contain any *.elm files"
            )),
            Ok(found) => files.extend(found),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                problems.push(format!("{shown}: No such file or directory"));
            }
            Err(e) => problems.push(format!("{shown}: {e}")),
        }
    }
    if problems.is_empty() {
        Ok(files)
    } else {
        Err(problems)
    }
}

/// What the options ask for, `None` for nothing at all, or why they
/// disagree; upstream's `determineWhatToDoFromConfig`.
fn task(options: &Options, files: Vec<PathBuf>) -> Result<Option<Task>, &'static str> {
    if options.stdin && !files.is_empty() {
        return Err("Too many input sources! Please only provide one of either INPUT or --stdin\n");
    }
    if !options.stdin && files.is_empty() {
        return Ok(None);
    }
    if options.validate {
        return match (&options.output, options.stdin) {
            (Some(_), _) => Err("Cannot use --output and --validate together\n"),
            (None, true) => Ok(Some(Task::ValidateStdin)),
            (None, false) => Ok(Some(Task::ValidateFiles(files))),
        };
    }
    Ok(Some(match (&options.output, options.stdin) {
        (None, true) => Task::StdinToStdout,
        (Some(output), true) => Task::StdinToFile(output.clone()),
        (None, false) => Task::FilesInPlace(files),
        (Some(output), false) if files.len() == 1 => {
            Task::FileToFile(files.into_iter().next().unwrap(), output.clone())
        }
        (Some(_), false) => {
            return Err(concat!(
                "Can't write to the OUTPUT path, because multiple .elm files have been specified.\n",
                "\n",
                "Please remove the --output argument. The .elm files in INPUT will be formatted in place.\n",
                "\n",
            ));
        }
    }))
}

/// Upstream's guess when `--elm-version` is missing: 0.18 for a project
/// with only an `elm-package.json`, else 0.19.
fn detect_version(vfs: &Vfs) -> ElmVersion {
    if vfs.is_file(Path::new(paths::ELM_PACKAGE_JSON)) && !vfs.is_file(Path::new(paths::ELM_JSON)) {
        ElmVersion::V0_18
    } else {
        ElmVersion::V0_19
    }
}

fn unsupported(version: ElmVersion, given: bool) -> &'static str {
    match (version, given) {
        (ElmVersion::V0_17, _) => {
            "nomi formats Elm 0.19 only; it does not support --elm-version=0.17.\n"
        }
        (_, true) => "nomi formats Elm 0.19 only; it does not support --elm-version=0.18.\n",
        (_, false) => concat!(
            "nomi formats Elm 0.19 only, and this directory has an elm-package.json but no ",
            "elm.json, which makes it Elm 0.18; pass --elm-version=0.19 to format it anyway.\n",
        ),
    }
}

/// Formats standard input and hands the result to `then`; messages go
/// to standard error, as standard output may hold the result.
fn stdin_to(
    console: &mut dyn Console,
    then: impl FnOnce(&mut dyn Console, String) -> bool,
) -> bool {
    let bytes = match console.read_all() {
        Ok(bytes) => bytes,
        Err(e) => {
            console.err(&format!("elm-format: <stdin>: {e}\n"));
            return false;
        }
    };
    match decode(bytes).and_then(|text| format_text(&text)) {
        Ok(text) => then(console, text),
        Err(at) => {
            console.err(&unable_to_parse("<STDIN>", at));
            false
        }
    }
}

/// Asks before overwriting `files`: `Ok(false)` if the answer is no, and
/// the message to fail with if standard input ends first.
fn approve(files: &[PathBuf], console: &mut dyn Console) -> Result<bool, String> {
    let mut prompt =
        String::from("This will overwrite the following files to use Elm's preferred style:\n\n");
    for file in files {
        prompt.push_str(&format!("    {}\n", file.display()));
    }
    prompt.push_str(concat!(
        "\n",
        "This cannot be undone! Make sure to back up these files before proceeding.\n",
        "\n",
        "Are you sure you want to overwrite these files with formatted versions? (y/n)\n",
        "\n",
    ));
    console.out(&prompt);
    loop {
        match console.read_line() {
            Ok(Some(line)) if line == "y" => return Ok(true),
            Ok(Some(line)) if line == "n" => return Ok(false),
            Ok(Some(_)) => console.out("Must type 'y' for yes or 'n' for no: "),
            Ok(None) => return Err("elm-format: <stdin>: hGetLine: end of file\n".to_owned()),
            Err(e) => return Err(format!("elm-format: <stdin>: {e}\n")),
        }
    }
}

/// Formats `files` in place on the pool, then reports on each in order.
fn in_place(vfs: &Vfs, files: Vec<PathBuf>, console: &mut dyn Console) -> bool {
    let outcomes = Pool::global().map(files, |file| {
        let formatted = read(vfs, &file)
            .map(|text| text.and_then(|text| format_text(&text).map(|f| (f != text, f))));
        let outcome = match formatted {
            Ok(Ok((false, _))) => Outcome::Unchanged,
            Ok(Ok((true, text))) => match vfs.write(&file, &text) {
                Ok(()) => Outcome::Formatted,
                Err(e) => Outcome::Failed(e),
            },
            Ok(Err(at)) => Outcome::Invalid(at),
            Err(e) => Outcome::Failed(e),
        };
        (file, outcome)
    });
    let mut succeeded = true;
    for (file, outcome) in outcomes {
        console.out(&processing(&file));
        match outcome {
            Outcome::Unchanged | Outcome::Formatted => {}
            Outcome::Invalid(at) => {
                console.out(&unable_to_parse(&file.display().to_string(), at));
                succeeded = false;
            }
            Outcome::Failed(e) => {
                console.err(&crashed(&file, &e));
                succeeded = false;
            }
        }
    }
    succeeded
}

/// The text of the file at `path`, or, where it isn't UTF-8, the
/// position of the syntax error that makes it.
fn read(vfs: &Vfs, path: &Path) -> io::Result<Result<String, ElmPosition>> {
    vfs.read_bytes(path).map(decode)
}

/// Whether `bytes` are a formatted module, or where they fail to parse.
fn is_formatted(bytes: Vec<u8>) -> Result<bool, ElmPosition> {
    let text = decode(bytes)?;
    format_text(&text).map(|formatted| formatted == text)
}

/// Writes a result to `path`, created if missing, as upstream writes
/// `--output` whether or not it changed.
fn write(vfs: &Vfs, path: &Path, text: &str, console: &mut dyn Console) -> bool {
    match vfs.create(path, text) {
        Ok(()) => true,
        Err(e) => {
            console.err(&crashed(path, &e));
            false
        }
    }
}

fn processing(file: &Path) -> String {
    format!("Processing file {}\n", file.display())
}

fn unable_to_parse(file: &str, at: ElmPosition) -> String {
    format!(
        "Unable to parse file {file}:{}:{} To see a detailed explanation, run elm make on the file.\n",
        at.row, at.column
    )
}

/// How upstream reports an I/O failure: the uncaught exception's text.
fn crashed(file: &Path, error: &io::Error) -> String {
    format!("elm-format: {}: {error}\n", file.display())
}

/// The `--validate` entry for a module: `formatted` says whether it is,
/// or where it fails to parse.
fn validation_entry(path: &str, formatted: Result<bool, ElmPosition>) -> Option<String> {
    let message = match formatted {
        Ok(true) => return None,
        Ok(false) => "File is not formatted with elm-format-0.8.8 --elm-version=0.19",
        Err(_) => "Error parsing the file",
    };
    Some(format!(
        "{{\"path\":{},\"message\":{}}}",
        json_string(path),
        json_string(message)
    ))
}

/// Upstream's `resultsToJsonString`, and the line break after it.
fn json_list(entries: &[String]) -> String {
    if entries.is_empty() {
        "[]\n".to_owned()
    } else {
        format!("[{}\n]\n", entries.join("\n,"))
    }
}

/// A JSON string as aeson writes it: only `"`, `\` and control characters
/// are escaped, `\n`, `\r` and `\t` by name.
fn json_string(text: &str) -> String {
    let mut json = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            c if c < ' ' => json.push_str(&format!("\\u{:04x}", c as u32)),
            c => json.push(c),
        }
    }
    json.push('"');
    json
}
