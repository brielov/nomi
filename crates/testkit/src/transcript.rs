//! Command-line transcripts: a case sets up a directory, runs a command in
//! it with some arguments and standard input, and its transcript records
//! the exit code, both output streams and every file left behind.
//!
//! A case file has sections, each a `=== ` header line followed by the
//! section's lines:
//!
//! - `=== args`: one argument per line;
//! - `=== stdin`: standard input, empty without the section;
//! - `=== file <path>`: a file to create, by a `/`-separated relative path;
//! - `=== note`: what the case is about; ignored.
//!
//! A transcript prints each recorded text with every line behind `|`, so
//! trailing spaces show, and a `\ no newline at end` line where the text
//! doesn't end in one.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// A parsed case file.
#[derive(Debug, Default)]
pub struct Case {
    pub args: Vec<String>,
    pub stdin: String,
    pub files: Vec<(String, String)>,
}

impl Case {
    pub fn parse(text: &str) -> Result<Case, String> {
        let mut case = Case::default();
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
        for (header, body) in sections {
            match header.split_once(' ') {
                None if header == "args" => case.args = body.lines().map(str::to_owned).collect(),
                None if header == "stdin" => case.stdin = body,
                None if header == "note" => {}
                Some(("file", path)) => case.files.push((path.to_owned(), body)),
                _ => return Err(format!("unknown section `=== {header}`")),
            }
        }
        Ok(case)
    }

    /// Creates the case's files under `dir`.
    pub fn create_files(&self, dir: &Path) -> io::Result<()> {
        for (path, text) in &self.files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().expect("a file in a directory"))?;
            fs::write(path, text)?;
        }
        Ok(())
    }

    /// Runs `command` on the case in `dir`, which holds its files, and
    /// records what happened.
    pub fn run(&self, mut command: Command, dir: &Path) -> io::Result<Transcript> {
        let mut child = command
            .args(&self.args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // A command may exit without reading its input.
        let _ = child.stdin.take().unwrap().write_all(self.stdin.as_bytes());
        let output = child.wait_with_output()?;
        let mut files = Vec::new();
        list(dir, "", &mut files)?;
        Ok(Transcript {
            exit: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            files,
        })
    }
}

/// What running a case did.
#[derive(Debug)]
pub struct Transcript {
    /// `None` if a signal ended the command.
    pub exit: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Every file in the case's directory afterwards, by path.
    pub files: Vec<(String, Vec<u8>)>,
}

impl Transcript {
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

fn section(out: &mut String, header: &str, bytes: &[u8]) {
    out.push_str(&format!("=== {header}\n"));
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
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            list(&entry.path(), &format!("{path}/"), files)?;
        } else {
            files.push((path, fs::read(entry.path())?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(Case::parse("=== argv\n").is_err());
    }

    #[test]
    fn transcripts_mark_lines_and_missing_newlines() {
        let transcript = Transcript {
            exit: Some(1),
            stdout: b"a \nb".to_vec(),
            stderr: Vec::new(),
            files: vec![("x".to_owned(), b"\n".to_vec())],
        };
        assert_eq!(
            transcript.render(),
            "=== exit 1\n=== stdout\n|a \n|b\n\\ no newline at end\n=== stderr\n=== file x\n|\n"
        );
    }
}
