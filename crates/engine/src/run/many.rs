//! `nomi run` of several entries at once: every one built first, then
//! each browser program served and each Bun program started with its
//! `args`, its lines shown under the name of its member, until they have
//! all ended, or, while a program is served, until nomi is stopped. One
//! that fails stops the others and gives nomi its code, and SIGINT or
//! SIGTERM stops them all within one grace. The watch runs its programs
//! here too, a single one on nomi's own streams.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use nomi_base::{Compat, Platform, Report};
use nomi_project::Entry;
use nomi_project::workspace::{Member, Workspace};
use nomi_registry::Http;

mod watch;

pub(super) use watch::watch;

use super::port::{Port, ports};
use super::{
    Caught, GRACE, Served, Signalled, Stderr, Stop, bun, exit_code, no_bun, not_a_module, send,
    serve, serving,
};
use crate::Context;
use crate::console::{self, Console};
use crate::make::has_ext;
use crate::paths::relative;
use crate::serve::{Reloads, Server};
use crate::workspace::{self, EntryBuilt, Making, Outcome};

/// Builds the entries of the members of `workspace` in `dirs`, then
/// serves each browser program among them on its port of [`ports`], and
/// runs each Bun program until every
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
    let printing: Vec<(&str, bool)> = (labels.iter().zip(&builds.entries))
        .map(|(label, built)| (label.as_str(), built.platform == Platform::Bun))
        .collect();
    let prefixes = prefixes_of(&printing);
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
    let mut running = Running::new(console, servers, Streams::Prefixed, Vec::new());
    let programs =
        (builds.entries.iter().zip(prefixes)).filter(|(b, _)| b.platform == Platform::Bun);
    for (built, prefix) in programs {
        if let Err(e) = running.start(built, prefix) {
            console::report(console, &no_bun(&e), Compat::LATEST);
            return running.drain(console, &mut caught, Some(Stop::Terminate), 1);
        }
    }
    running.supervise(console, &mut caught)
}

/// Why `built`, its output named from `root`, can't run: a Bun program
/// written to something other than an ES module.
fn unrunnable(built: &EntryBuilt<'_>, root: &Path) -> Option<Report> {
    let output = relative(&built.output, root);
    (built.platform == Platform::Bun && !has_ext(".mjs", &output.to_string_lossy()))
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
        let port = port.expect("a browser program doesn't run on Bun by its manifest");
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

/// What comes before a program's lines: `[label] `, padded, in a color
/// of its own on a terminal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Prefix {
    /// `[label]`, or nothing where the program's streams are nomi's.
    label: String,
    pad: usize,
    color: u8,
}

impl Prefix {
    /// The prefix on a stream that is a terminal where `terminal`.
    fn on(&self, terminal: bool) -> String {
        if self.label.is_empty() {
            String::new()
        } else if terminal {
            format!(
                "\x1b[{}m{}\x1b[0m{:pad$} ",
                self.color,
                self.label,
                "",
                pad = self.pad
            )
        } else {
            format!("{}{:pad$} ", self.label, "", pad = self.pad)
        }
    }
}

/// The prefix of each of `labels`, padded to the longest of those whose
/// program prints, as each says: a browser program's label is only in
/// nomi's lines.
fn prefixes_of(labels: &[(&str, bool)]) -> Vec<Prefix> {
    let width = (labels.iter())
        .filter(|(_, printing)| *printing)
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    (labels.iter().enumerate())
        .map(|(i, (label, _))| Prefix {
            label: format!("[{label}]"),
            pad: width.saturating_sub(label.chars().count()),
            color: COLORS[i % COLORS.len()],
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

/// How the programs' lines and nomi's own reach nomi's streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Streams {
    /// Each program's lines read and shown after its prefix, and nomi's
    /// own after `[nomi]`, as several programs share them.
    Prefixed,
    /// The one program's streams are nomi's, and nomi's lines are bare.
    Shared,
}

impl Streams {
    /// What comes before a line nomi says itself.
    fn nomi(self) -> &'static str {
        match self {
            Streams::Prefixed => "[nomi] ",
            Streams::Shared => "",
        }
    }
}

/// A line a program printed, after its prefix.
struct Line {
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
    prefix: Prefix,
    child: Child,
    ended: Option<ExitStatus>,
}

/// Why programs are killed rather than left to stop.
#[derive(Clone, Copy)]
enum Killing {
    /// A second SIGINT asked for it.
    Asked,
    /// They didn't stop within [`GRACE`], which nomi says.
    Overdue,
}

/// The programs started and the servers serving the browser programs.
struct Running {
    /// The programs started that haven't ended or whose end hasn't been
    /// taken, by the ids [`Running::start`] gave them.
    programs: BTreeMap<usize, Program>,
    /// The id the next program takes.
    next: usize,
    servers: Vec<Server>,
    streams: Streams,
    /// The arguments every program takes in place of its entry's, where
    /// any are given.
    args: Vec<OsString>,
    /// Whether nomi's standard output and error are terminals, which
    /// prefixes are colored on.
    terminals: (bool, bool),
    send: Sender<Line>,
    lines: Receiver<Line>,
}

/// How often the programs are looked at while no line comes.
const TICK: Duration = Duration::from_millis(10);

/// How long nomi reads what the programs printed once they have all
/// ended: a process one started may hold its pipes open far longer.
const LINGER: Duration = Duration::from_secs(1);

impl Running {
    /// No program yet, beside `servers`; programs to start with `args`
    /// where given, their lines shown on `console` as `streams` says.
    fn new(
        console: &dyn Console,
        servers: Vec<Server>,
        streams: Streams,
        args: Vec<OsString>,
    ) -> Running {
        let (send, lines) = mpsc::channel();
        Running {
            programs: BTreeMap::new(),
            next: 0,
            servers,
            streams,
            args,
            terminals: (console.out_is_terminal(), console.err_is_terminal()),
            send,
            lines,
        }
    }

    /// Starts `built` under `bun` with its entry's `args`, or those
    /// every program takes, its standard output and error read line by
    /// line and shown after `prefix` where they are piped; its id.
    fn start(&mut self, built: &EntryBuilt<'_>, prefix: Prefix) -> std::io::Result<usize> {
        let entry: Vec<OsString>;
        let args = if self.args.is_empty() {
            entry = built.entry.args.iter().map(OsString::from).collect();
            &entry
        } else {
            &self.args
        };
        let mut bun = bun(&built.output, args);
        if self.streams == Streams::Prefixed {
            bun.stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }
        let mut child = bun.spawn()?;
        if let Some(out) = child.stdout.take() {
            read(
                out,
                prefix.on(self.terminals.0),
                Stream::Out,
                self.send.clone(),
            );
        }
        if let Some(err) = child.stderr.take() {
            read(
                err,
                prefix.on(self.terminals.1),
                Stream::Err,
                self.send.clone(),
            );
        }
        let id = self.next;
        self.next += 1;
        let program = Program {
            prefix,
            child,
            ended: None,
        };
        self.programs.insert(id, program);
        Ok(id)
    }

    /// Shows the programs' lines until they have all ended and nothing is
    /// served, stopping the others when one fails or nomi is sent SIGINT
    /// or SIGTERM; the code to exit with.
    fn supervise(mut self, console: &mut dyn Console, caught: &mut Caught) -> u8 {
        loop {
            self.show(console);
            if let Some(signal) = caught.take() {
                return self.drain(console, caught, signal.passed_on(), signal.stop.code());
            }
            self.reap();
            let failed = (self.programs.values())
                .filter_map(|p| p.ended.map(exit_code))
                .find(|&code| code != 0);
            if let Some(code) = failed {
                return self.drain(console, caught, Some(Stop::Terminate), code);
            }
            if self.programs.values().all(|p| p.ended.is_some()) && self.servers.is_empty() {
                return self.drain(console, caught, None, 0);
            }
        }
    }

    /// Stops every program still running, as [`Running::stop`] does, then
    /// the servers, and shows the last lines, those read within
    /// [`LINGER`] or until nomi is sent another signal; `code`, or 130 if
    /// SIGINT comes meanwhile.
    fn drain(
        mut self,
        console: &mut dyn Console,
        caught: &mut Caught,
        signal: Option<Stop>,
        mut code: u8,
    ) -> u8 {
        let every: Vec<usize> = self.programs.keys().copied().collect();
        if let Some(Stop::Interrupt) = self.stop(console, caught, &every, signal).map(|s| s.stop) {
            code = Stop::Interrupt.code();
        }
        let Running {
            servers,
            send,
            lines,
            ..
        } = self;
        drop(servers);
        drop(send);
        // The readers end as the programs' pipes close, unless a process
        // a program started holds one: they are left to end with nomi.
        let until = Instant::now() + LINGER;
        while let Some(left) = until.checked_duration_since(Instant::now())
            && caught.take().is_none()
        {
            match lines.recv_timeout(left.min(TICK)) {
                Ok(line) => print(console, &line),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        code
    }

    /// Stops the programs `chosen` names that are still running, sending
    /// each `signal` at once where given (a terminal's reaches them
    /// already), waits for them together within [`GRACE`] and kills those
    /// left, and forgets them; the first signal nomi is sent meanwhile,
    /// SIGINT killing them at once.
    fn stop(
        &mut self,
        console: &mut dyn Console,
        caught: &mut Caught,
        chosen: &[usize],
        signal: Option<Stop>,
    ) -> Option<Signalled> {
        if let Some(signal) = signal {
            for id in chosen {
                if let Some(program) = self.programs.get(id).filter(|p| p.ended.is_none()) {
                    let _ = send(&program.child, signal);
                }
            }
        }
        let start = Instant::now();
        let mut first = None;
        self.reap();
        let ended = |programs: &BTreeMap<usize, Program>| {
            (chosen.iter()).all(|id| programs.get(id).is_none_or(|p| p.ended.is_some()))
        };
        while !ended(&self.programs) {
            if let Some(signal) = caught.take() {
                first = first.or(Some(signal));
                if signal.stop == Stop::Interrupt {
                    self.kill(console, chosen, Killing::Asked);
                    break;
                }
            }
            if start.elapsed() >= GRACE {
                self.kill(console, chosen, Killing::Overdue);
                break;
            }
            self.show(console);
            self.reap();
        }
        for id in chosen {
            self.programs.remove(id);
        }
        first
    }

    /// Kills the programs `chosen` names that are still running, as
    /// `killing` says.
    fn kill(&mut self, console: &mut dyn Console, chosen: &[usize], killing: Killing) {
        for id in chosen {
            let Some(program) = self.programs.get_mut(id).filter(|p| p.ended.is_none()) else {
                continue;
            };
            match killing {
                Killing::Asked => {}
                Killing::Overdue => console.err(&format!(
                    "{}The program did not stop within {} seconds, so I killed it.\n",
                    program.prefix.on(self.terminals.1),
                    GRACE.as_secs()
                )),
            }
            let _ = program.child.kill();
            program.ended = program.child.wait().ok();
        }
    }

    /// Notes how each program that has ended since last looked at ended.
    fn reap(&mut self) {
        for program in self.programs.values_mut().filter(|p| p.ended.is_none()) {
            program.ended = program.child.try_wait().ok().flatten();
        }
    }

    /// How the program `id` ended, if it has, forgetting it then.
    fn ended(&mut self, id: usize) -> Option<ExitStatus> {
        let ended = self.programs.get(&id)?.ended?;
        self.programs.remove(&id);
        Some(ended)
    }

    /// Shows the lines that come within a [`TICK`].
    fn show(&self, console: &mut dyn Console) {
        if let Ok(line) = self.lines.recv_timeout(TICK) {
            print(console, &line);
            for line in self.lines.try_iter() {
                print(console, &line);
            }
        }
    }
}

/// Shows `line` on the stream it came from.
fn print(console: &mut dyn Console, line: &Line) {
    match line.stream {
        Stream::Out => console.out(&line.text),
        Stream::Err => console.err(&line.text),
    }
}

/// Reads `pipe` line by line into `send`, each after `prefix`, on a
/// thread of its own that ends as the pipe closes or nomi does.
fn read(pipe: impl Read + Send + 'static, prefix: String, stream: Stream, send: Sender<Line>) {
    thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut bytes = Vec::new();
        while pipe
            .read_until(b'\n', &mut bytes)
            .is_ok_and(|read| read > 0)
        {
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_suffix('\n').unwrap_or(&text);
            let text = text.strip_suffix('\r').unwrap_or(text);
            let text = format!("{prefix}{text}\n");
            if send.send(Line { stream, text }).is_err() {
                break;
            }
            bytes.clear();
        }
    });
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

    #[test]
    fn a_prefix_is_padded_to_the_programs_that_print_and_colored_only_on_a_terminal() {
        let prefixes = prefixes_of(&[("apps/api", true), ("apps/landing", false)]);
        assert_eq!(prefixes[0].on(false), "[apps/api] ");
        assert_eq!(prefixes[0].on(true), "\x1b[36m[apps/api]\x1b[0m ");
        assert_eq!(prefixes[1].on(false), "[apps/landing] ");
        let prefixes = prefixes_of(&[("apps/api", true), ("apps/landing", true)]);
        assert_eq!(prefixes[0].on(false), "[apps/api]     ");
        assert_eq!(Prefix::default().on(true), "");
    }
}
