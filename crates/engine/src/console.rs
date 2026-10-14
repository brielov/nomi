//! The terminal a command talks to: its standard streams, or a test's.

use std::collections::VecDeque;
use std::io::{self, BufRead, IsTerminal, Read, Write};

use nomi_base::diagnostic::Doc;
use nomi_base::diagnostic::render::{Styles, WIDTH, render};
use nomi_base::{Compat, Report};
use nomi_project::report::ElmCommand;

use crate::report::one_line;

/// The terminal a run talks to.
pub trait Console {
    /// Prints `text` on standard output.
    fn out(&mut self, text: &str);
    /// Prints `text` on standard error.
    fn err(&mut self, text: &str);
    /// The next line of standard input, without its `\n`, once standard
    /// output is flushed; `None` at its end.
    fn read_line(&mut self) -> io::Result<Option<String>>;
    /// The rest of standard input.
    fn read_all(&mut self) -> io::Result<Vec<u8>>;
    /// Whether standard output is a terminal, where reports show in color.
    fn out_is_terminal(&self) -> bool {
        false
    }
    /// Whether standard error is a terminal.
    fn err_is_terminal(&self) -> bool {
        false
    }
}

/// The process's own standard streams. Failed writes are ignored, as
/// there is nowhere left to report them.
#[derive(Debug, Default)]
pub struct StdConsole;

impl Console for StdConsole {
    fn out(&mut self, text: &str) {
        let _ = io::stdout().write_all(text.as_bytes());
    }

    fn err(&mut self, text: &str) {
        let _ = io::stderr().write_all(text.as_bytes());
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.ends_with('\n') {
            line.pop();
        }
        Ok(Some(line))
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn out_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }

    fn err_is_terminal(&self) -> bool {
        io::stderr().is_terminal()
    }
}

/// Whose words a command speaks: elm's, when nomi is invoked as `elm`, or
/// nomi's own, which name nomi's commands where elm names its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personality {
    Elm,
    Nomi,
}

impl Personality {
    /// The command line that runs elm's `command`: `elm install`, or
    /// `nomi add`.
    pub fn command(self, command: ElmCommand) -> String {
        match self {
            Personality::Elm => format!("elm {}", command.elm_name()),
            Personality::Nomi => {
                let nomi = match command {
                    ElmCommand::Init => "new",
                    ElmCommand::Install => "add",
                    ElmCommand::Make => "build",
                };
                format!("nomi {nomi}")
            }
        }
    }
}

/// A console that keeps what is printed and reads what it was given, as
/// tests run commands.
#[derive(Debug, Default)]
pub struct Captured {
    /// What `read_line` reads, a line at a time, before the input ends.
    pub lines: VecDeque<io::Result<String>>,
    /// What `read_all` reads.
    pub input: Vec<u8>,
    pub out: String,
    pub err: String,
}

impl Console for Captured {
    fn out(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn err(&mut self, text: &str) {
        self.err.push_str(text);
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        self.lines.pop_front().transpose()
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        Ok(std::mem::take(&mut self.input))
    }
}

/// `Help.toStdout`: `doc` on standard output, styled on a terminal.
pub(crate) fn print(console: &mut dyn Console, doc: &Doc, compat: Compat) {
    let styles = styles(console.out_is_terminal());
    console.out(&render(doc, compat, WIDTH, styles));
}

/// `Exit.toStderr`: `report` on standard error, styled on a terminal.
pub fn report(console: &mut dyn Console, report: &Report, compat: Compat) {
    report_doc(console, &report.to_doc(), compat);
}

/// `Exit.toStderr` of a document that isn't one report, as a list of
/// modules' reports is.
pub fn report_doc(console: &mut dyn Console, doc: &Doc, compat: Compat) {
    let styles = styles(console.err_is_terminal());
    console.err(&render(doc, compat, WIDTH, styles));
}

fn styles(terminal: bool) -> Styles {
    if terminal {
        Styles::Ansi
    } else {
        Styles::Plain
    }
}

/// An answer to [`ask`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Yes,
    No,
    /// Standard input ended first.
    Ended,
}

/// `Reporting.ask`: `question`, then the answer, asked again until it is
/// yes or no. An empty answer is yes.
pub(crate) fn ask(console: &mut dyn Console, question: &Doc, compat: Compat) -> io::Result<Answer> {
    print(console, question, compat);
    loop {
        match console.read_line()?.as_deref() {
            None => return Ok(Answer::Ended),
            Some("" | "Y" | "y") => return Ok(Answer::Yes),
            Some("n") => return Ok(Answer::No),
            Some(_) => console.out("Must type 'y' for yes or 'n' for no: "),
        }
    }
}

/// What a command that changes `elm.json` does once `answer`ed: `change`
/// on a yes; on a no nothing, as it says; and where input ended, what a
/// question without an answer says. Whether it succeeded: a decline is a
/// success.
pub(crate) fn answered(
    console: &mut dyn Console,
    personality: Personality,
    answer: Answer,
    change: impl FnOnce(&mut dyn Console) -> Result<(), Report>,
) -> Result<bool, Report> {
    match answer {
        Answer::Yes => change(console).map(|()| true),
        Answer::No => {
            console.out("Okay, I did not change anything!\n");
            Ok(true)
        }
        Answer::Ended => {
            no_answer(console, personality);
            Ok(false)
        }
    }
}

/// [`ask`]s `question` about a change to `elm.json`, and once
/// [`answered`] yes makes it with `change` and says so.
pub(crate) fn confirm(
    console: &mut dyn Console,
    question: &Doc,
    compat: Compat,
    personality: Personality,
    change: impl FnOnce() -> Result<(), Report>,
) -> Result<bool, Report> {
    let answer = ask(console, question, compat)
        .map_err(|e| one_line("PROBLEM READING ANSWER", &e.to_string()))?;
    answered(console, personality, answer, |console| {
        change()?;
        console.out("Success!\n");
        Ok(())
    })
}

/// What a question without an answer says. Upstream's `getLine` throws,
/// and the program dies with GHC's message for it.
pub(crate) fn no_answer(console: &mut dyn Console, personality: Personality) {
    match personality {
        Personality::Elm => console.err("elm: <stdin>: hGetLine: end of file\n"),
        Personality::Nomi => console.err("error: standard input ended before an answer\n"),
    }
}

/// Whether a command that ended with `result` succeeded, its problem
/// reported in `compat`'s words if it had one.
pub(crate) fn reported(
    console: &mut dyn Console,
    result: Result<bool, Report>,
    compat: Compat,
) -> bool {
    result.unwrap_or_else(|problem| {
        report(console, &problem, compat);
        false
    })
}
