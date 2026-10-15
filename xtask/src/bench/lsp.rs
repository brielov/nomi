//! The language server's part of the scoreboard: nomi's
//! server against the pinned elm-language-server, each a child process
//! spoken to over its standard streams as an editor would, on corpus apps
//! of three sizes.
//!
//! Each workload opens an app's entry module with a type error added,
//! which both servers report on open (time to first diagnostics), then
//! saves it alternately fixed and broken again (save to diagnostics),
//! then, saved fixed once more, asks for completions in it (completion).
//! Diagnostics come on open and save for both, as the replay suite's
//! settings have them. CPU time is the server's own and its children's,
//! and memory its peak resident set, from `/proc`, so this part runs on
//! Linux.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use crossbeam_channel::RecvTimeoutError;
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::Url;
use serde_json::{Value, json};

use super::measure::Cost;
use super::{Row, Setup, Table};
use crate::references::References;

/// The apps, small to large.
const APPS: &[&str] = &["todomvc", "elm-spa-example", "exosphere"];

/// Fixed and broken saves after the first diagnostics, and completions.
const SAVES: usize = 6;
const COMPLETIONS: usize = 6;

/// A line that doesn't type-check, added at the end of the module.
const BROKEN: &str = "\n\nnomiBenchBroken : Int\nnomiBenchBroken =\n    1 + \"one\"\n";

/// How long an answer may take before the run counts as hung.
const LIMIT: Duration = Duration::from_secs(300);

/// Silence that counts as the server having settled.
const QUIET: Duration = Duration::from_millis(1500);

/// The clock ticks a second `/proc/<pid>/stat` counts CPU time in:
/// Linux's `USER_HZ`, which is 100 everywhere it runs.
const TICKS: u64 = 100;

pub(crate) fn table(root: &Path, refs: &References, setup: &Setup) -> Result<Table> {
    ensure!(
        cfg!(target_os = "linux"),
        "the language server's part reads /proc, so runs on Linux"
    );
    let cache = super::elm::nomi_cache(setup)?;
    let scratch = setup.scratch.join("lsp");
    let tools = crate::corpus::lsp::server(refs, &scratch)?;
    let mut rows = Vec::new();
    for app in APPS {
        let project = crate::corpus::lsp::project(root, &format!("app/{app}"))?;
        let entry = entry(root, app)?;
        let mut costs = Vec::new();
        for server in [Server::Upstream, Server::Nomi] {
            let work = scratch.join("work").join(app);
            crate::corpus::lsp::copy(&project, &work)?;
            let work = fs::canonicalize(&work)?;
            let mut command = match server {
                Server::Nomi => {
                    let mut command = Command::new(setup.nomi("nomi"));
                    command.arg("lsp").env("NOMI_CACHE", &cache);
                    command
                }
                Server::Upstream => tools.server_command(),
            };
            command.current_dir(&work);
            crate::oracle::sandbox::offline(&mut command);
            let mut settings = nomi_testkit::lsp::default_settings();
            if server == Server::Upstream {
                settings.extend(tools.settings(&scratch.join("no-elm-review")));
            }
            let module = Module::read(work.join(&entry))?;
            costs.push(session(command, &work, &module, Value::Object(settings))?);
        }
        let [upstream, nomi] = <[Measured; 2]>::try_from(costs)
            .ok()
            .context("two servers")?;
        for (name, upstream, nomi) in [
            ("first diagnostics", upstream.first, nomi.first),
            ("save to diagnostics", upstream.save, nomi.save),
            ("completion", upstream.completion, nomi.completion),
        ] {
            rows.push(Row {
                name: format!("{app}: {name}"),
                upstream,
                nomi,
            });
        }
        eprintln!("  {app}");
    }
    Ok(Table {
        title: "language server".to_owned(),
        about: format!(
            "nomi's server against elm-language-server, each spoken to over its standard \
             streams as an editor would, on corpus apps small to large. Once the server has \
             started and gone quiet, a session opens the app's entry module with a type error added, which both report on open (first \
             diagnostics); saves it fixed and broken again, {SAVES} times (save to \
             diagnostics); and, saved fixed once more, asks for completions in it, \
             {COMPLETIONS} times. The times are \
             the best of each, from the message sent to the answer received. CPU time is the \
             server's and its child processes', and memory the peak of their resident sets \
             together, sampled every 20 ms from the session's start to the end of that part. Both run offline over the same package sources, with \
             the replay suite's settings: diagnostics on open and save."
        ),
        upstream: "elm-language-server".to_owned(),
        rows,
        refused: Vec::new(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Server {
    Upstream,
    Nomi,
}

/// What one server's session cost, by part.
struct Measured {
    first: Cost,
    save: Cost,
    completion: Cost,
}

/// A part's best wall time and its CPU time, before its memory is known.
struct Timed {
    wall: Duration,
    cpu: Duration,
}

/// The app's first entry module, by path in its root.
fn entry(root: &Path, app: &str) -> Result<String> {
    let apps = crate::corpus::apps::load(root)?.app;
    let app = (apps.iter())
        .find(|a| a.name == app)
        .with_context(|| format!("{app} isn't a corpus app"))?;
    app.entries
        .first()
        .cloned()
        .with_context(|| format!("{} has no entries", app.name))
}

/// The module a session edits: where it is, as a path and a URI, and its
/// text fixed, as the app has it, and broken.
struct Module {
    path: PathBuf,
    uri: Url,
    clean: String,
    broken: String,
}

/// Which of a module's texts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Text {
    Fixed,
    Broken,
}

impl Module {
    fn read(path: PathBuf) -> Result<Module> {
        let clean = fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
        let broken = format!("{}{BROKEN}", clean.trim_end());
        let uri = Url::from_file_path(&path).map_err(|()| anyhow::anyhow!("a path"))?;
        Ok(Module {
            path,
            uri,
            clean,
            broken,
        })
    }

    fn text(&self, text: Text) -> &str {
        match text {
            Text::Fixed => &self.clean,
            Text::Broken => &self.broken,
        }
    }
}

/// Runs `command`, a server, through a session on `module` in `work`.
fn session(
    mut command: Command,
    work: &Path,
    module: &Module,
    settings: Value,
) -> Result<Measured> {
    fs::write(&module.path, &module.broken)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the server")?;
    let pid = child.id();
    // The tree's resident memory, sampled: elm-language-server's `elm
    // make` runs come and go between the parts' ends.
    let peak = Arc::new(AtomicU64::new(0));
    let sampling = Arc::new(AtomicBool::new(true));
    let sampler = {
        let (peak, sampling) = (peak.clone(), sampling.clone());
        std::thread::spawn(move || {
            while sampling.load(Ordering::Relaxed) {
                peak.fetch_max(resident(pid), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };
    let connection = nomi_testkit::lsp::child_connection(
        child.stdin.take().context("its stdin")?,
        child.stdout.take().context("its stdout")?,
    );
    let mut client = Client {
        connection,
        settings,
        next: 0,
        version: 1,
    };
    let result = (|| -> Result<Measured> {
        client.initialize(work)?;
        let cost = |timed: Timed| Cost {
            wall: timed.wall,
            cpu: timed.cpu,
            memory: peak.load(Ordering::Relaxed),
        };
        let first = cost(Timed {
            wall: client.open(module)?,
            cpu: cpu(pid),
        });
        let save = cost(client.saves(module, pid)?);
        let completion = cost(client.completions(module, pid)?);
        let _ = client.request("shutdown", Value::Null);
        client.notify("exit", Value::Null);
        Ok(Measured {
            first,
            save,
            completion,
        })
    })();
    sampling.store(false, Ordering::Relaxed);
    let _ = sampler.join();
    nomi_testkit::lsp::reap(child);
    result
}

/// Just after the first letter of the first name on each of a few lines
/// of the module's bodies, a name being typed: no keyword.
fn completion_positions(text: &str) -> Vec<(usize, usize)> {
    const KEYWORDS: &[&str] = &["let", "in", "case", "of", "if", "then", "else"];
    let mut found = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let indented = content.len() - content.trim_start().len();
        let rest = content.trim_start();
        let word: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if indented >= 4
            && word.starts_with(|c: char| c.is_ascii_lowercase())
            && !KEYWORDS.contains(&word.as_str())
        {
            found.push((line, indented + 1));
        }
        if found.len() == COMPLETIONS {
            break;
        }
    }
    found
}

/// The server's CPU time so far: its process's and its descendants', as
/// a version manager's `node` runs the real one as its child. It counts
/// children that have exited too.
fn cpu(pid: u32) -> Duration {
    let ticks: u64 = (tree(pid).into_iter())
        .map(|pid| stat(pid).map_or(0, |(_, ticks)| ticks))
        .sum();
    Duration::from_millis(ticks * 1000 / TICKS)
}

/// The resident memory of `pid` and its descendants now, in KiB.
fn resident(pid: u32) -> u64 {
    tree(pid).into_iter().map(resident_kib).sum()
}

/// `pid` and its descendants.
fn tree(pid: u32) -> Vec<u32> {
    let parents: Vec<(u32, u32)> = (fs::read_dir("/proc").into_iter().flatten().flatten())
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|p| Some((p, stat(p)?.0)))
        .collect();
    let mut tree = vec![pid];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        tree.extend(
            parents
                .iter()
                .filter(|(_, p)| *p == parent)
                .map(|(pid, _)| *pid),
        );
        i += 1;
    }
    tree
}

/// A process's own resident memory, `VmRSS` in `/proc/<pid>/status`, in
/// KiB; 0 without one.
fn resident_kib(pid: u32) -> u64 {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    (status.lines())
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// A process's parent, and its CPU time in ticks, its exited children's
/// included.
fn stat(pid: u32) -> Option<(u32, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the command's name, in parentheses: the parent is the 4th
    // field, utime the 14th, stime the 15th, cutime and cstime the 16th
    // and 17th.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let parent = fields.get(1)?.parse().ok()?;
    let ticks = fields
        .get(11..15)?
        .iter()
        .map(|f| f.parse::<u64>().unwrap_or(0))
        .sum();
    Some((parent, ticks))
}

/// The editor end of a session.
struct Client {
    connection: Connection,
    settings: Value,
    /// The last request's id.
    next: i32,
    /// The edited module's version.
    version: i32,
}

impl Client {
    /// Starts the server on the project at `work`, then waits until it
    /// has gone quiet, as elm-language-server takes no notice of a file
    /// opened while it reads the project.
    fn initialize(&mut self, work: &Path) -> Result<()> {
        let root_uri = Url::from_directory_path(work).map_err(|()| anyhow::anyhow!("a path"))?;
        self.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": nomi_testkit::lsp::client_capabilities(),
                "initializationOptions": self.settings,
                "workspaceFolders": [{ "uri": root_uri, "name": "bench" }],
            }),
        )?;
        self.notify("initialized", json!({}));
        self.notify(
            "workspace/didChangeConfiguration",
            json!({ "settings": { "elmLS": self.settings } }),
        );
        self.settle()
    }

    /// Opens `module`, broken, and waits for its error: the time to the
    /// first diagnostics.
    fn open(&self, module: &Module) -> Result<Duration> {
        let start = Instant::now();
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": module.uri, "languageId": "elm", "version": 1, "text": module.broken } }),
        );
        self.diagnostics(&module.uri, Text::Broken)?;
        Ok(start.elapsed())
    }

    /// Saves `module` fixed and broken in turn, [`SAVES`] times, then
    /// fixed again unmeasured, for the completions: the best time from a
    /// save to its diagnostics, and the CPU time a save takes.
    fn saves(&mut self, module: &Module, pid: u32) -> Result<Timed> {
        let mut text = Text::Broken;
        let mut best = Duration::MAX;
        let before = cpu(pid);
        for _ in 0..SAVES {
            text = match text {
                Text::Fixed => Text::Broken,
                Text::Broken => Text::Fixed,
            };
            let start = Instant::now();
            self.save(module, text)?;
            best = best.min(start.elapsed());
        }
        let after = cpu(pid);
        if text == Text::Broken {
            self.save(module, Text::Fixed)?;
        }
        Ok(Timed {
            wall: best,
            cpu: (after - before) / SAVES as u32,
        })
    }

    /// Changes `module` to `text` and saves it, and waits for the
    /// diagnostics that follow.
    fn save(&mut self, module: &Module, text: Text) -> Result<()> {
        self.version += 1;
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": module.uri, "version": self.version },
                "contentChanges": [{ "text": module.text(text) }],
            }),
        );
        fs::write(&module.path, module.text(text))?;
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": module.uri } }),
        );
        self.diagnostics(&module.uri, text)
    }

    /// Asks for completions in `module`, fixed, [`COMPLETIONS`] times:
    /// the best time to an answer, and the CPU time one takes.
    fn completions(&mut self, module: &Module, pid: u32) -> Result<Timed> {
        let positions = completion_positions(&module.clean);
        ensure!(
            !positions.is_empty(),
            "nowhere to complete in {}",
            module.path.display()
        );
        let mut best = Duration::MAX;
        let before = cpu(pid);
        for (line, character) in positions.iter().cycle().take(COMPLETIONS) {
            let start = Instant::now();
            self.request(
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": module.uri },
                    "position": { "line": line, "character": character },
                }),
            )?;
            best = best.min(start.elapsed());
        }
        let after = cpu(pid);
        Ok(Timed {
            wall: best,
            cpu: (after - before) / COMPLETIONS as u32,
        })
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self
            .connection
            .sender
            .send(Message::Notification(Notification::new(
                method.to_owned(),
                params,
            )));
    }

    /// Sends a request and waits for its answer, which must not be an
    /// error.
    fn request(&mut self, method: &str, params: Value) -> Result<()> {
        self.next += 1;
        let id = RequestId::from(self.next);
        self.connection.sender.send(Message::Request(Request::new(
            id.clone(),
            method.to_owned(),
            params,
        )))?;
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Message::Response(response) = self.next_message(deadline)?
                && response.id == id
            {
                if let Some(error) = response.error {
                    bail!("{method} failed: {} ({})", error.message, error.code);
                }
                return Ok(());
            }
        }
    }

    /// Waits until diagnostics for `uri` are published as `text` has
    /// them: with errors for the broken text, none for the fixed.
    fn diagnostics(&self, uri: &Url, text: Text) -> Result<()> {
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Message::Notification(n) = self.next_message(deadline)?
                && n.method == "textDocument/publishDiagnostics"
                && n.params["uri"].as_str() == Some(uri.as_str())
            {
                let errors = (n.params["diagnostics"].as_array().into_iter().flatten())
                    .any(|d| d["severity"] == 1);
                if errors == (text == Text::Broken) {
                    return Ok(());
                }
            }
        }
    }

    /// Until the server has said nothing for [`QUIET`], within the limit.
    fn settle(&self) -> Result<()> {
        let deadline = Instant::now() + LIMIT;
        loop {
            let quiet = Instant::now() + QUIET;
            ensure!(quiet <= deadline, "the server never went quiet");
            if self.receive(quiet)?.is_none() {
                return Ok(());
            }
        }
    }

    /// The next message from the server before `deadline`.
    fn next_message(&self, deadline: Instant) -> Result<Message> {
        self.receive(deadline)?.context("the server took too long")
    }

    /// The next message from the server before `deadline`, answering its
    /// requests meanwhile; none if it says nothing by then. Its leaving is
    /// an error.
    fn receive(&self, deadline: Instant) -> Result<Option<Message>> {
        loop {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(None);
            };
            let message = match self.connection.receiver.recv_timeout(left) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => bail!("the server left"),
            };
            if let Message::Request(request) = &message {
                let result = match request.method.as_str() {
                    "workspace/configuration" => {
                        let items = request.params["items"].as_array().map_or(1, Vec::len);
                        json!(vec![self.settings.clone(); items])
                    }
                    _ => Value::Null,
                };
                let response = Response::new_ok(request.id.clone(), result);
                let _ = self.connection.sender.send(Message::Response(response));
                continue;
            }
            return Ok(Some(message));
        }
    }
}
