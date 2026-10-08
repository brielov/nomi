//! The terminal a command talks to: its standard streams, or a test's.

use std::io::{self, BufRead, IsTerminal, Read, Write};

use nomi_base::diagnostic::Doc;
use nomi_base::diagnostic::render::{Styles, WIDTH, render};
use nomi_base::{Compat, Report};

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
    /// The program's name, as messages begin with it.
    pub fn program(self) -> &'static str {
        match self {
            Personality::Elm => "elm",
            Personality::Nomi => "nomi",
        }
    }

    /// The command line that runs elm's `command`: `elm install`, or
    /// `nomi add`.
    pub fn command(self, command: &str) -> String {
        let nomi = match command {
            "init" => "new",
            "install" => "add",
            "make" => "build",
            "remove" => "rm",
            other => other,
        };
        match self {
            Personality::Elm => format!("elm {command}"),
            Personality::Nomi => format!("nomi {nomi}"),
        }
    }
}

/// `Help.toStdout`: `doc` on standard output, styled on a terminal.
pub fn print(console: &mut dyn Console, doc: &Doc, compat: Compat) {
    let styles = styles(console.out_is_terminal());
    console.out(&render(doc, compat, WIDTH, styles));
}

/// `Exit.toStderr`: `report` on standard error, styled on a terminal.
pub fn report(console: &mut dyn Console, report: &Report, compat: Compat) {
    let styles = styles(console.err_is_terminal());
    console.err(&render(&report.to_doc(), compat, WIDTH, styles));
}

fn styles(terminal: bool) -> Styles {
    if terminal {
        Styles::Ansi
    } else {
        Styles::Plain
    }
}

/// `Reporting.ask`: `question`, then the answer, asked again until it is
/// yes or no. An empty answer is yes. `None` when standard input ends
/// first.
pub fn ask(console: &mut dyn Console, question: &Doc, compat: Compat) -> io::Result<Option<bool>> {
    print(console, question, compat);
    loop {
        match console.read_line()?.as_deref() {
            None => return Ok(None),
            Some("" | "Y" | "y") => return Ok(Some(true)),
            Some("n") => return Ok(Some(false)),
            Some(_) => console.out("Must type 'y' for yes or 'n' for no: "),
        }
    }
}

/// What a question without an answer says. Upstream's `getLine` throws,
/// and the program dies with GHC's message for it.
pub fn no_answer(console: &mut dyn Console, personality: Personality) {
    match personality {
        Personality::Elm => console.err("elm: <stdin>: hGetLine: end of file\n"),
        Personality::Nomi => console.err("error: standard input ended before an answer\n"),
    }
}
