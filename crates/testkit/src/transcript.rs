//! Command-line transcripts: a case sets up a directory, runs a command in
//! it with some arguments and standard input, and its transcript records
//! the exit code, both output streams and every file left behind.
//!
//! A case file has sections, each a `=== ` header line followed by the
//! section's lines:
//!
//! - `=== args`: one argument per line, where `$ROOT` stands for the
//!   directory the case runs in;
//! - `=== stdin`: standard input, empty without the section;
//! - `=== file <path>`: a file to create, by a `/`-separated relative path;
//! - `=== bytes <path>`: one whose bytes aren't all text, written with
//!   `\xHH` for each byte that isn't printable ASCII and `\\` for a
//!   backslash;
//! - `=== link <path>`: a symbolic link to the target on its one line;
//! - `=== note`: what the case is about; ignored.
//!
//! A file's last line may be `\ no newline at end`, which drops the
//! newline before it, as a transcript prints it.
//!
//! A transcript prints each recorded text with every line behind `|`, so
//! trailing spaces show, and a `\ no newline at end` line where the text
//! doesn't end in one.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, UNIX_EPOCH};

use nomi_base::Compat;

use crate::process::run_within;

/// A parsed case file.
#[derive(Clone, Debug, Default)]
pub struct Case {
    /// The command's arguments, `$ROOT` standing for its directory.
    pub args: Vec<String>,
    /// The command's standard input.
    pub stdin: String,
    /// The text files to create, by path, with their text.
    pub files: Vec<(String, String)>,
    /// The `bytes` sections' files, decoded.
    pub binaries: Vec<(String, Vec<u8>)>,
    /// Symbolic links, by path, to their targets.
    pub links: Vec<(String, String)>,
}

/// The line that ends a text without a newline.
const NO_NEWLINE: &str = "\\ no newline at end\n";

/// How long, in bytes, the path of the directory the oracle runs a case
/// in is: elm wraps some messages by the length of the paths they name,
/// so a replay makes its directory as long, with
/// [`padded_dir`](crate::padded_dir).
const ROOT_LENGTH: usize = 96;

impl Case {
    /// The case a case file holds.
    pub fn parse(text: &str) -> Result<Case, String> {
        let mut case = Case::default();
        for (header, body) in sections(text)? {
            match header.split_once(' ') {
                None if header == "args" => case.args = body.lines().map(str::to_owned).collect(),
                None if header == "stdin" => case.stdin = body,
                None if header == "note" => {}
                Some(("file", path)) => case.files.push((path.to_owned(), unmarked(body))),
                Some(("bytes", path)) => {
                    let bytes = decode(&unmarked(body))
                        .ok_or_else(|| format!("a bad escape in `=== bytes {path}`"))?;
                    case.binaries.push((path.to_owned(), bytes));
                }
                Some(("link", path)) => case
                    .links
                    .push((path.to_owned(), body.trim_end().to_owned())),
                _ => return Err(format!("unknown section `=== {header}`")),
            }
        }
        Ok(case)
    }

    /// Creates the case's files under `dir`.
    pub fn create_files(&self, dir: &Path) -> io::Result<()> {
        let texts = (self.files.iter()).map(|(path, text)| (path, text.as_bytes()));
        let binaries = (self.binaries.iter()).map(|(path, bytes)| (path, bytes.as_slice()));
        for (path, bytes) in texts.chain(binaries) {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().expect("a file in a directory"))?;
            fs::write(path, bytes)?;
        }
        for (path, target) in &self.links {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().expect("a link in a directory"))?;
            symlink(target, &path)?;
        }
        Ok(())
    }

    /// Dates the case's text files under `dir` a second apart, in the
    /// case's order from 2001, so what sorts by modification time, as
    /// elm's terminal reports do, sorts as the case lists them and not as
    /// the file system's clock ticked.
    fn date_files(&self, dir: &Path) -> io::Result<()> {
        let start = UNIX_EPOCH + Duration::from_secs(978_307_200);
        for (i, (path, _)) in self.files.iter().enumerate() {
            let file = fs::File::options().write(true).open(dir.join(path))?;
            file.set_modified(start + Duration::from_secs(i as u64))?;
        }
        Ok(())
    }

    /// Lays the case's files out, dated, in a directory under `work`
    /// whose canonical path is [`ROOT_LENGTH`] bytes long, as the
    /// recordings were made; gives that directory.
    pub fn lay_out_padded(&self, work: &Path) -> io::Result<PathBuf> {
        let root = crate::padded_dir(&fs::canonicalize(work)?, ROOT_LENGTH)?;
        self.create_files(&root)?;
        self.date_files(&root)?;
        Ok(root)
    }

    /// The releases a replay runs the case as: each nomi emulates for an
    /// application whose `elm-version` names one, else the newest, as
    /// nomi builds a package.
    pub fn releases(&self) -> &'static [Compat] {
        let pins = (self.files.iter()).any(|(path, text)| {
            path == "elm.json"
                && text.contains(APPLICATION)
                && (Compat::ALL.iter()).any(|r| text.contains(&elm_version(*r)))
        });
        if pins {
            &Compat::ALL
        } else {
            &[Compat::LATEST]
        }
    }

    /// The case with `default` as its `elm.json` if it has none.
    pub fn with_manifest(&self, default: &str) -> Case {
        let mut case = self.clone();
        if !case.files.iter().any(|(path, _)| path == "elm.json") {
            (case.files).push(("elm.json".to_owned(), default.to_owned()));
        }
        case
    }

    /// The case with its application's pinned `elm-version` set to
    /// `release`, as the oracle ran it.
    pub fn as_release(mut self, release: Compat) -> Case {
        let to = elm_version(release);
        for (path, text) in &mut self.files {
            if path == "elm.json" && text.contains(APPLICATION) {
                for pinned in Compat::ALL {
                    *text = text.replace(&elm_version(pinned), &to);
                }
            }
        }
        self
    }

    /// The case's arguments, with `$ROOT` standing for `dir`.
    fn args_in(&self, dir: &Path) -> Vec<String> {
        let root = dir.to_string_lossy();
        (self.args.iter())
            .map(|a| a.replace("$ROOT", &root))
            .collect()
    }

    /// Runs `command` on the case in `dir`, which holds its files, and
    /// records what happened.
    pub fn run(&self, command: Command, dir: &Path) -> io::Result<Transcript> {
        self.run_ending(command, dir, None)
    }

    /// [`Case::run`], killing the command once it has run for `limit`, as
    /// a command that hangs ends: by a signal.
    pub fn run_within(
        &self,
        command: Command,
        dir: &Path,
        limit: Duration,
    ) -> io::Result<Transcript> {
        self.run_ending(command, dir, Some(limit))
    }

    /// `command` run on the case in `dir`, with its output piped and the
    /// case's input fed as it reads, killed past `limit` if one is given.
    fn run_ending(
        &self,
        mut command: Command,
        dir: &Path,
        limit: Option<Duration>,
    ) -> io::Result<Transcript> {
        command
            .args(self.args_in(dir))
            .current_dir(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (Ok(output) | Err(output)) = run_within(&mut command, self.stdin.as_bytes(), limit)?;
        Transcript::after(output.status, output.stdout, output.stderr, dir)
    }
}

/// What an `elm.json` of an application says.
const APPLICATION: &str = "\"type\": \"application\"";

/// An `elm.json`'s `elm-version` field, naming `release`.
fn elm_version(release: Compat) -> String {
    format!("\"elm-version\": \"{release}\"")
}

/// The case files in `dir`, `<name>.case`, by name, parsed, in name
/// order.
pub fn cases(dir: &Path) -> Vec<(String, Case)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .and_then(|entries| entries.map(|entry| Ok(entry?.path())).collect())
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    paths.retain(|path| path.extension().is_some_and(|e| e == "case"));
    paths.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));
    (paths.iter())
        .map(|path| {
            let name = path.file_stem().expect("a case file").to_string_lossy();
            let text =
                fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let case = Case::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (name.into_owned(), case)
        })
        .collect()
}

/// Runs each case in `cases` that `run` takes, which gives its
/// transcript, and fails, showing the differences, unless each is the
/// transcript of the same name in `expected`, `<name>.txt`, read once the
/// case has run. How many cases ran.
pub fn check_transcripts(
    cases: &Path,
    expected: &Path,
    mut run: impl FnMut(&str, &Case) -> Option<String>,
) -> usize {
    let mut ran = 0;
    let mut failures = Vec::new();
    for (name, case) in self::cases(cases) {
        let Some(actual) = run(&name, &case) else {
            continue;
        };
        ran += 1;
        let path = expected.join(format!("{name}.txt"));
        let expected =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Err(both) = matching(&expected, &actual) {
            failures.push(format!("{name}:\n{both}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {ran} transcripts differ:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
    ran
}

/// Whether `actual` is the transcript `expected`, and if not, both.
pub fn matching(expected: &str, actual: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("--- expected\n{expected}--- actual\n{actual}"))
    }
}

/// The transcript an oracle recorded for `release`, among the
/// `recordings` of a case: each distinct one follows a `### ` line naming
/// the elms that gave it, as `### elm 0.19.1, elm 0.19.2` does.
pub fn recorded(recordings: &str, release: Compat) -> Option<&str> {
    let elm = format!("elm {release}");
    (recordings.split("### ").skip(1)).find_map(|group| {
        let (elms, transcript) = group.split_once('\n')?;
        elms.split(", ").any(|e| e == elm).then_some(transcript)
    })
}

/// Whether elm died making `transcript`, by a signal or an internal
/// error, where nomi reports what it finds (`DECISIONS.md`, 2026-10-08).
pub fn crashed(transcript: &str) -> bool {
    transcript.starts_with("=== killed")
        || transcript.contains("bypassed the normal error reporting process")
}

/// The `=== ` sections of `text`, each a header and the lines after it.
fn sections(text: &str) -> Result<Vec<(&str, String)>, String> {
    let mut sections: Vec<(&str, String)> = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(header) = line.strip_prefix("=== ") {
            sections.push((header.trim_end_matches('\n'), String::new()));
        } else if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
        } else {
            return Err(format!("text before the first section: {line:?}"));
        }
    }
    Ok(sections)
}

/// `text` without a closing [`NO_NEWLINE`] and the newline before it.
fn unmarked(mut text: String) -> String {
    if text.ends_with(NO_NEWLINE) {
        text.truncate(text.len() - NO_NEWLINE.len());
        text.pop();
    }
    text
}

/// The bytes a `bytes` section's text stands for.
fn decode(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut rest = text.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        rest = after;
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match rest {
            [b'\\', after @ ..] => {
                bytes.push(b'\\');
                rest = after;
            }
            [b'x', hi, lo, after @ ..] => {
                let digits = [*hi, *lo];
                let hex = std::str::from_utf8(&digits).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
                rest = after;
            }
            _ => return None,
        }
    }
    Some(bytes)
}

#[cfg(unix)]
fn symlink(target: &str, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(not(unix))]
fn symlink(target: &str, path: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, path)
}

/// What running a case did.
#[derive(Debug)]
pub struct Transcript {
    /// `None` if a signal ended the command.
    pub exit: Option<i32>,
    /// What the command wrote to its standard output.
    pub stdout: Vec<u8>,
    /// What the command wrote to its standard error.
    pub stderr: Vec<u8>,
    /// Every file in the case's directory afterwards, by path.
    pub files: Vec<(String, Vec<u8>)>,
}

/// How a transcript shows a symbolic link: this, then its target.
const LINK: &str = "symbolic link to ";

/// Files past this many bytes show by their hash.
const LARGE: usize = 16 * 1024;

impl Transcript {
    /// What a command that ended with `status`, having written `stdout`
    /// and `stderr`, left in `dir`.
    fn after(
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        dir: &Path,
    ) -> io::Result<Transcript> {
        let mut files = Vec::new();
        list(dir, "", &mut files)?;
        Ok(Transcript {
            exit: status.code(),
            stdout,
            stderr,
            files,
        })
    }

    /// The transcript of `case` run in `dir` as compared with elm's: what
    /// is left in `elm-stuff/` is elm's own, files the case wrote that
    /// are as it wrote them are left out, a large file shows by its
    /// SHA-256, and `dir` reads `$ROOT`.
    pub fn compared(mut self, case: &Case, dir: &Path) -> String {
        let written = |path: &str, bytes: &[u8]| {
            let text = (case.files.iter()).any(|(p, t)| p == path && t.as_bytes() == bytes);
            let binary = (case.binaries.iter()).any(|(p, b)| p == path && b == bytes);
            let link = (case.links.iter())
                .any(|(p, t)| p == path && bytes == format!("{LINK}{t}\n").as_bytes());
            text || binary || link
        };
        self.files
            .retain(|(path, bytes)| !path.starts_with("elm-stuff/") && !written(path, bytes));
        for (_, bytes) in &mut self.files {
            if bytes.len() > LARGE {
                let hash = crate::sha256_hex(bytes);
                *bytes = format!("{} bytes, SHA-256 {hash}\n", bytes.len()).into_bytes();
            }
        }
        self.render().replace(&*dir.to_string_lossy(), "$ROOT")
    }

    /// The transcript [`Transcript::render`] wrote as `text`, its texts
    /// read back as UTF-8.
    pub fn parse(text: &str) -> Result<Transcript, String> {
        let mut transcript = Transcript {
            exit: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            files: Vec::new(),
        };
        let mut sections = sections(text)?.into_iter();
        match sections.next() {
            Some(("killed", _)) => {}
            Some((header, _)) => {
                let code = header
                    .strip_prefix("exit ")
                    .and_then(|code| code.parse().ok());
                transcript.exit = Some(code.ok_or_else(|| format!("`=== {header}` is no exit"))?);
            }
            None => return Err("an empty transcript".to_owned()),
        }
        for (header, body) in sections {
            let lines = body.split_inclusive('\n');
            let text: String = lines.map(|l| l.strip_prefix('|').unwrap_or(l)).collect();
            let bytes = unmarked(text).into_bytes();
            match header.split_once(' ') {
                None if header == "stdout" => transcript.stdout = bytes,
                None if header == "stderr" => transcript.stderr = bytes,
                Some(("file", path)) => transcript.files.push((path.to_owned(), bytes)),
                _ => return Err(format!("unknown section `=== {header}`")),
            }
        }
        Ok(transcript)
    }

    /// The transcript as text: the exit, both output streams, then each
    /// file, as sections.
    pub fn render(&self) -> String {
        let mut out = match self.exit {
            Some(code) => format!("=== exit {code}\n"),
            None => "=== killed\n".to_owned(),
        };
        section(&mut out, "stdout", &self.stdout);
        section(&mut out, "stderr", &self.stderr);
        for (path, bytes) in &self.files {
            section(&mut out, &format!("file {path}"), bytes);
        }
        out
    }
}

/// Writes `bytes` to `out` as a transcript's section under `header`.
fn section(out: &mut String, header: &str, bytes: &[u8]) {
    let _ = writeln!(out, "=== {header}");
    let text = String::from_utf8_lossy(bytes);
    for line in text.split_inclusive('\n') {
        out.push('|');
        out.push_str(line);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        out.push_str("\n\\ no newline at end\n");
    }
}

/// The files under `dir`, by `/`-separated paths below `prefix`, in the
/// order of their names.
fn list(dir: &Path, prefix: &str, files: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = format!("{prefix}{}", entry.file_name().to_string_lossy());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let target = fs::read_link(entry.path())?;
            let text = format!("{LINK}{}\n", target.to_string_lossy());
            files.push((path, text.into_bytes()));
        } else if kind.is_dir() {
            list(&entry.path(), &format!("{path}/"), files)?;
        } else {
            files.push((path, fs::read(entry.path())?));
        }
    }
    Ok(())
}

/// elm-test's output with what changes from run to run, or machine to
/// machine, put plainly: the case's directory and `ELM_HOME` as `{dir}` and
/// `{home}`, durations as 0, the help's core count as 8, and a stack
/// trace's Node version and lines and columns in the JavaScript as 0.
pub fn normalize_elm_test(output: &[u8], dir: &str, home: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(output)
        .replace(dir, "{dir}")
        .replace(home, "{home}");
    let text = digits_after(&text, "\"duration\":\"", "0");
    let text = digits_after(&text, " time=\"", "0");
    let text = digits_after(&text, "Duration: ", "0");
    // Colored, its label and number apart.
    let text = digits_after(&text, "Duration: \u{1b}[22m", "0");
    let text = digits_after(&text, "logical CPU cores, ", "8");
    // A stack trace's Node and its lines in the JavaScript.
    let text = digits_after(&text, "Node.js v", "0");
    let text = digits_after(&text, ".js:", "0");
    let text = digits_after(&text, ".js:0:", "0");
    text.into_bytes()
}

/// `text` with the number after each `marker`, digits and points, as `with`.
fn digits_after(text: &str, marker: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(marker) {
        out.push_str(&rest[..at + marker.len()]);
        rest = &rest[at + marker.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if end > 0 {
            out.push_str(with);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_command_that_never_reads_its_input_still_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let case = Case {
            args: vec!["-c".to_owned(), "sleep 30".to_owned()],
            stdin: "x".repeat(1 << 20),
            ..Case::default()
        };
        let started = Instant::now();
        case.run_within(Command::new("sh"), dir.path(), Duration::from_millis(200))
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn cases_parse_by_section() {
        let case = Case::parse(
            "=== note\nwhy\n=== args\n--yes\nsrc\n=== stdin\ny\n=== file src/A.elm\nmodule A\n\n",
        )
        .unwrap();
        assert_eq!(case.args, ["--yes", "src"]);
        assert_eq!(case.stdin, "y\n");
        assert_eq!(
            case.files,
            [("src/A.elm".to_owned(), "module A\n\n".to_owned())]
        );
        assert!(Case::parse("args\n").is_err());
        let case = Case::parse(
            "=== bytes a\n\\xff\\\\x\n\\ no newline at end\n=== file b\nb\n\\ no newline at end\n=== link c\nb\n",
        )
        .unwrap();
        assert_eq!(case.binaries, [("a".to_owned(), b"\xff\\x".to_vec())]);
        assert_eq!(case.files, [("b".to_owned(), "b".to_owned())]);
        assert_eq!(case.links, [("c".to_owned(), "b".to_owned())]);
        assert!(Case::parse("=== bytes a\n\\q\n").is_err());
        assert!(Case::parse("=== argv\n").is_err());
    }

    #[test]
    fn recordings_name_the_releases_that_gave_them() {
        let text = "### elm 0.19.1, elm 0.19.2\n=== exit 0\n### elm 0.19.3\n=== killed\n";
        assert_eq!(recorded(text, Compat::V0_19_2), Some("=== exit 0\n"));
        let last = recorded(text, Compat::V0_19_3).unwrap();
        assert!(crashed(last) && !crashed("=== exit 0\n"));
    }

    #[test]
    fn a_pinned_application_runs_as_each_release() {
        let case = |kind: &str| {
            let elm_json = format!(r#"{{"type": "{kind}", "elm-version": "0.19.1"}}"#);
            Case::parse(&format!("=== file elm.json\n{elm_json}\n")).unwrap()
        };
        assert_eq!(case("application").releases(), Compat::ALL);
        assert_eq!(case("package").releases(), [Compat::LATEST]);
        let as_latest = case("application").as_release(Compat::V0_19_3);
        assert_eq!(
            as_latest.files[0].1,
            "{\"type\": \"application\", \"elm-version\": \"0.19.3\"}\n"
        );
    }

    #[test]
    fn a_default_manifest_fills_in_only_where_one_is_missing() {
        let with = |text: &str| Case::parse(text).unwrap().with_manifest("default").files;
        let own = [("elm.json".to_owned(), "own\n".to_owned())];
        assert_eq!(with("=== file elm.json\nown\n"), own);
        assert_eq!(with(""), [("elm.json".to_owned(), "default".to_owned())]);
    }

    #[test]
    fn transcripts_mark_lines_and_missing_newlines() {
        let transcript = Transcript {
            exit: Some(1),
            stdout: b"a \nb".to_vec(),
            stderr: Vec::new(),
            files: vec![("x".to_owned(), b"\n".to_vec())],
        };
        let text = transcript.render();
        assert_eq!(
            text,
            "=== exit 1\n=== stdout\n|a \n|b\n\\ no newline at end\n=== stderr\n=== file x\n|\n"
        );
        let parsed = Transcript::parse(&text).unwrap();
        assert_eq!(parsed.exit, Some(1));
        assert_eq!(parsed.stdout, b"a \nb");
        assert_eq!(parsed.files, transcript.files);
        assert!(Transcript::parse("=== killed\n").unwrap().exit.is_none());
        assert!(Transcript::parse("=== exit x\n").is_err());
    }
}
