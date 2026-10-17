//! `nomi run` of several entries at once: every one built first, then
//! each browser program served and each Node program started with its
//! `args`, its lines shown under the name of its member, until they have
//! all ended, or, while a program is served, until nomi is stopped. One
//! that fails stops the others and gives nomi its code, and SIGINT or
//! SIGTERM stops them all within one grace.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nomi_base::{Compat, Platform, Report};
use nomi_project::Entry;
use nomi_project::workspace::{Member, Workspace};
use nomi_registry::Http;

mod watch;

pub(super) use watch::watch;

use super::port::{Port, ports};
use super::{
    Caught, GRACE, Served, Stderr, Stop, exit_code, no_node, node, not_a_module, send, serve,
    serving,
};
use crate::Context;
use crate::console::{self, Console};
use crate::make::has_ext;
use crate::paths::relative;
use crate::serve::{Reloads, Server};
use crate::workspace::{self, EntryBuilt, Making, Outcome};

/// Builds the entries of the members of `workspace` in `dirs`, then
/// serves each browser program among them on its port of [`ports`], and
/// runs each Node program until every
/// one has ended, or while one is served until nomi is stopped, showing
/// on standard error how the builds went. The code to exit with: the
/// first failed program's, 130 or 143 when nomi was stopped by SIGINT or
/// SIGTERM, 0 when they all succeeded, or 1 if they could not all be
/// built and started.
pub(super) fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    workspace: &Workspace,
    dirs: &[PathBuf],
) -> u8 {
    let builds = workspace::builds(cx, &mut Stderr(console), workspace, dirs, Making::Run);
    if !(builds.outcomes.iter()).all(|(_, outcome)| matches!(outcome, Outcome::Built(_))) {
        return 1;
    }
    let root = workspace.root();
    if let Some(report) = builds.entries.iter().find_map(|b| unrunnable(b, root)) {
        console::report(console, &report, Compat::LATEST);
        return 1;
    }
    let labels: Vec<String> = (builds.entries.iter())
        .map(|built| label_of(built.member, built.entry))
        .collect();
    let prefixes = prefixes_of(&labels, console.err_is_terminal());
    let mut caught = match Caught::new() {
        Ok(caught) => caught,
        Err(e) => {
            console.err(&format!("I could not watch for signals: {e}\n"));
            return 1;
        }
    };
    let ports = ports(builds.entries.iter().map(|built| built.entry));
    let Some(servers) = servers(console, root, &builds.entries, &labels, &ports) else {
        return 1;
    };
    let mut running = Running::new(servers);
    let programs =
        (builds.entries.iter().zip(prefixes)).filter(|(b, _)| b.platform == Platform::Node);
    for (built, prefix) in programs {
        if let Err(e) = running.start(built, prefix) {
            console::report(console, &no_node(&e), Compat::LATEST);
            return running.drain(console, &mut caught, Some(Stop::Terminate), 1);
        }
    }
    running.supervise(console, &mut caught)
}

/// Why `built`, its output named from `root`, can't run: a Node program
/// written to something other than an ES module.
fn unrunnable(built: &EntryBuilt<'_>, root: &Path) -> Option<Report> {
    let output = relative(&built.output, root);
    (built.platform == Platform::Node && !has_ext(".mjs", &output.to_string_lossy()))
        .then(|| not_a_module(&built.entry.main, &output.display().to_string()))
}

/// Serves each browser program of `entries`, as [`served`] does, saying
/// where by its label of `labels`, each on its port of `ports`; the
/// servers, or `None` once what went wrong is shown.
fn servers(
    console: &mut dyn Console,
    root: &Path,
    entries: &[EntryBuilt<'_>],
    labels: &[String],
    ports: &[Option<Port>],
) -> Option<Vec<Server>> {
    let browser = (entries.iter().zip(labels).zip(ports))
        .filter(|((b, _), _)| b.platform == Platform::Browser);
    let mut servers = Vec::new();
    for ((built, label), port) in browser {
        let port = port.expect("a browser program doesn't run on Node by its manifest");
        match served(root, built, port, ports, Reloads::Never) {
            Ok(server) => {
                console.err(&format!("[nomi] {}", serving(label, &server, port)));
                servers.push(server);
            }
            Err(report) => {
                console::report(console, &report, Compat::LATEST);
                return None;
            }
        }
    }
    Some(servers)
}

/// Serves the browser program `built`, of the workspace at `root`, on
/// `port`, one of `ports`, its pages listening for reloads as `reloads`
/// says.
fn served(
    root: &Path,
    built: &EntryBuilt<'_>,
    port: Port,
    ports: &[Option<Port>],
    reloads: Reloads,
) -> Result<Server, Report> {
    let dir = root.join(&built.member.dir);
    serve(Served {
        main: &built
            .member
            .dir
            .join(&built.entry.main)
            .display()
            .to_string(),
        output: &built.output,
        module: &built.module,
        static_dir: built.entry.static_dir.as_ref().map(|d| dir.join(d)),
        port,
        ports,
        reloads,
        from: root,
        dir: &dir,
    })
}

/// The name of `entry` of `member`: the member by its directory from the
/// root, and the entry by its `main`'s name where the member has several.
fn label_of(member: &Member, entry: &Entry) -> String {
    let several = (member.manifest.as_ref()).is_some_and(|m| m.entries.len() > 1);
    label(&member.dir, several, &entry.main)
}

/// The prefix of each of `labels`, `[label] `, padded to the longest; in
/// color where `color`.
fn prefixes_of(labels: &[String], color: bool) -> Vec<String> {
    let width = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    (labels.iter().enumerate())
        .map(|(i, label)| {
            let pad = " ".repeat(width - label.chars().count());
            if color {
                let code = COLORS[i % COLORS.len()];
                format!("\x1b[{code}m[{label}]\x1b[0m{pad} ")
            } else {
                format!("[{label}]{pad} ")
            }
        })
        .collect()
}

/// The ANSI colors prefixes take in turn: cyan, magenta, yellow, green
/// and blue, which neither reports' red nor plain text use.
const COLORS: [u8; 5] = [36, 35, 33, 32, 34];

/// The name of the entry whose `main` is `main` in the member in `dir`,
/// from the root, which has `several` entries or one.
fn label(dir: &Path, several: bool, main: &str) -> String {
    let dir = dir.display().to_string();
    let stem = Path::new(main)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    match (dir.is_empty(), several) {
        (true, true) => stem.into_owned(),
        (true, false) => ".".to_owned(),
        (false, true) => format!("{dir}:{stem}"),
        (false, false) => dir,
    }
}

/// A line a program printed.
struct Line {
    /// The program's place in [`Running::programs`].
    from: usize,
    stream: Stream,
    text: String,
}

/// Which of a program's streams a line came from.
#[derive(Clone, Copy)]
enum Stream {
    Out,
    Err,
}

/// A program started, and how it ended, once it has.
struct Program {
    prefix: String,
    child: Child,
    ended: Option<ExitStatus>,
}

/// The programs started, the threads reading their lines, and the
/// servers serving the browser programs.
struct Running {
    /// Every program started, whether it has ended or not, each where
    /// its lines name it.
    programs: Vec<Program>,
    readers: Vec<JoinHandle<()>>,
    servers: Vec<Server>,
    send: Sender<Line>,
    lines: Receiver<Line>,
}

/// How often the programs are looked at while no line comes.
const TICK: Duration = Duration::from_millis(10);

impl Running {
    /// No program yet, beside `servers`.
    fn new(servers: Vec<Server>) -> Running {
        let (send, lines) = mpsc::channel();
        Running {
            programs: Vec::new(),
            readers: Vec::new(),
            servers,
            send,
            lines,
        }
    }

    /// Starts `built` under `node` with its entry's `args`, its standard
    /// output and error read line by line, shown after `prefix`; its
    /// place among the programs.
    fn start(&mut self, built: &EntryBuilt<'_>, prefix: String) -> std::io::Result<usize> {
        let args: Vec<OsString> = built.entry.args.iter().map(OsString::from).collect();
        let mut child = node(&built.output, &args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let from = self.programs.len();
        let out = child.stdout.take().expect("standard output is piped");
        let err = child.stderr.take().expect("standard error is piped");
        self.readers
            .push(read(out, from, Stream::Out, self.send.clone()));
        self.readers
            .push(read(err, from, Stream::Err, self.send.clone()));
        self.programs.push(Program {
            prefix,
            child,
            ended: None,
        });
        Ok(from)
    }

    /// Shows the programs' lines until they have all ended and nothing is
    /// served, stopping the others when one fails or nomi is sent SIGINT
    /// or SIGTERM; the code to exit with.
    fn supervise(mut self, console: &mut dyn Console, caught: &mut Caught) -> u8 {
        loop {
            self.show(console);
            if let Some((signal, sent)) = caught.take() {
                return self.drain(console, caught, sent.then_some(signal), signal.code());
            }
            self.reap();
            let failed = (self.programs.iter())
                .filter_map(|p| p.ended.map(exit_code))
                .find(|&code| code != 0);
            if let Some(code) = failed {
                return self.drain(console, caught, Some(Stop::Terminate), code);
            }
            if self.ended() && self.servers.is_empty() {
                return self.drain(console, caught, None, 0);
            }
        }
    }

    /// Stops every program still running, as [`Running::stop`] does, then
    /// the servers, and shows every last line; `code`, or 130 if SIGINT
    /// comes meanwhile.
    fn drain(
        mut self,
        console: &mut dyn Console,
        caught: &mut Caught,
        signal: Option<Stop>,
        mut code: u8,
    ) -> u8 {
        let every: Vec<usize> = (0..self.programs.len()).collect();
        if let Some((Stop::Interrupt, _)) = self.stop(console, caught, &every, signal) {
            code = Stop::Interrupt.code();
        }
        let Running {
            programs,
            readers,
            servers,
            send,
            lines,
        } = self;
        drop(servers);
        drop(send);
        // The readers end as the programs' pipes close.
        while let Ok(line) = lines.recv() {
            print(&programs, console, &line);
        }
        for reader in readers {
            let _ = reader.join();
        }
        code
    }

    /// Stops the programs `chosen` names that are still running, sending
    /// each `signal` at once where given (a terminal's reaches them
    /// already), waits for them together within [`GRACE`] and kills those
    /// left; the first signal nomi is sent meanwhile, SIGINT killing
    /// them at once.
    fn stop(
        &mut self,
        console: &mut dyn Console,
        caught: &mut Caught,
        chosen: &[usize],
        signal: Option<Stop>,
    ) -> Option<(Stop, bool)> {
        if let Some(signal) = signal {
            let running = (self.programs.iter_mut().enumerate())
                .filter(|(i, p)| p.ended.is_none() && chosen.contains(i));
            for child in running.map(|(_, p)| &mut p.child) {
                let _ = send(child, signal);
            }
        }
        let start = Instant::now();
        let mut first = None;
        self.reap();
        while !chosen.iter().all(|&i| self.programs[i].ended.is_some()) {
            if let Some(signal) = caught.take() {
                first = first.or(Some(signal));
                if let (Stop::Interrupt, _) = signal {
                    self.kill(console, chosen, true);
                    break;
                }
            }
            if start.elapsed() >= GRACE {
                self.kill(console, chosen, false);
                break;
            }
            self.show(console);
            self.reap();
        }
        first
    }

    /// Kills the programs `chosen` names that are still running, saying
    /// so unless `interrupted` asked for it.
    fn kill(&mut self, console: &mut dyn Console, chosen: &[usize], interrupted: bool) {
        for &i in chosen {
            let program = &mut self.programs[i];
            if program.ended.is_some() {
                continue;
            }
            if !interrupted {
                console.err(&format!(
                    "{}The program did not stop within {} seconds, so I killed it.\n",
                    program.prefix,
                    GRACE.as_secs()
                ));
            }
            let _ = program.child.kill();
            program.ended = program.child.wait().ok();
        }
    }

    /// Notes how each program that has ended since last looked at ended.
    fn reap(&mut self) {
        for program in self.programs.iter_mut().filter(|p| p.ended.is_none()) {
            program.ended = program.child.try_wait().ok().flatten();
        }
    }

    /// Whether every program has ended.
    fn ended(&self) -> bool {
        self.programs.iter().all(|p| p.ended.is_some())
    }

    /// Shows the lines that come within a [`TICK`].
    fn show(&self, console: &mut dyn Console) {
        match self.lines.recv_timeout(TICK) {
            Ok(line) => {
                print(&self.programs, console, &line);
                for line in self.lines.try_iter() {
                    print(&self.programs, console, &line);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Every program's pipes are closed; their ends come soon.
            Err(RecvTimeoutError::Disconnected) => thread::sleep(TICK),
        }
    }
}

/// Shows `line`, of one of `programs`, after its prefix.
fn print(programs: &[Program], console: &mut dyn Console, line: &Line) {
    let text = format!("{}{}\n", programs[line.from].prefix, line.text);
    match line.stream {
        Stream::Out => console.out(&text),
        Stream::Err => console.err(&text),
    }
}

/// Reads `pipe` line by line into `send`, on a thread of its own.
fn read(
    pipe: impl Read + Send + 'static,
    from: usize,
    stream: Stream,
    send: Sender<Line>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut bytes = Vec::new();
        while pipe
            .read_until(b'\n', &mut bytes)
            .is_ok_and(|read| read > 0)
        {
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_suffix('\n').unwrap_or(&text);
            let text = text.strip_suffix('\r').unwrap_or(text).to_owned();
            if send.send(Line { from, stream, text }).is_err() {
                break;
            }
            bytes.clear();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_is_named_by_its_member_and_by_its_main_where_the_member_has_several() {
        let cases = [
            ("apps/api", false, "src/Main.elm", "apps/api"),
            ("apps/api", true, "src/Admin.elm", "apps/api:Admin"),
            ("", false, "src/Main.elm", "."),
            ("", true, "src/Admin.elm", "Admin"),
        ];
        for (dir, several, main, named) in cases {
            assert_eq!(label(Path::new(dir), several, main), named, "{dir} {main}");
        }
    }
}
