//! The language server's part of the scoreboard (plan 9.22): nomi's
//! server against the pinned elm-language-server, each a child process
//! spoken to over its standard streams as an editor would, on corpus apps
//! of three sizes.
//!
//! Each workload opens an app's entry module with a type error added,
//! which both servers report on open (time to first diagnostics), then
//! saves it alternately fixed and broken again (save to diagnostics),
//! then asks for completions in it (completion). Diagnostics come on open
//! and save for both, as the replay suite's settings have them. CPU time
//! is the server's own and its children's, and memory its peak resident
//! set, from `/proc`, so this part runs on Linux.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
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

pub fn table(root: &Path, refs: &References, setup: &Setup) -> Result<Table> {
    ensure!(
        cfg!(target_os = "linux"),
        "the language server's part reads /proc, so runs on Linux"
    );
    let cache = super::elm::nomi_cache(setup)?;
    let (home, packages) = crate::corpus::lsp::home_for_lsp()?;
    let scratch = setup.scratch.join("lsp");
    let tools = crate::oracle::lsp::tools(
        refs,
        crate::oracle::lsp::server_path()?,
        &scratch,
        home,
        packages,
    )?;
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
                Server::Upstream => {
                    let (node, script, path, home) = tools.command();
                    let mut command = Command::new(node);
                    command
                        .arg(script)
                        .arg("--stdio")
                        .env("PATH", path)
                        .env("ELM_HOME", home);
                    command
                }
            };
            command.current_dir(&work);
            crate::oracle::sandbox::offline(&mut command);
            let mut settings = nomi_testkit::lsp::default_settings();
            if server == Server::Upstream
                && let Value::Object(paths) = tools.settings(&scratch.join("no-elm-review"))
            {
                settings.extend(paths);
            }
            costs.push(session(command, &work, &entry, Value::Object(settings))?);
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
             diagnostics); and asks for completions in it, {COMPLETIONS} times. The times are \
             the best of each, from the message sent to the answer received. CPU time is the \
             server's and its child processes', and memory the sum of their peak resident \
             sets, as each stands at the end of that part. Both run offline over the same package sources, with \
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

/// Runs `command`, a server, through a session on `entry` in `work`.
fn session(mut command: Command, work: &Path, entry: &str, settings: Value) -> Result<Measured> {
    let path = work.join(entry);
    let clean = fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let broken = format!("{}{BROKEN}", clean.trim_end());
    fs::write(&path, &broken)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the server")?;
    let pid = child.id();
    let connection = nomi_testkit::lsp::child_connection(
        child.stdin.take().context("its stdin")?,
        child.stdout.take().context("its stdout")?,
    );
    let mut client = Client {
        connection,
        settings,
        next: 0,
    };
    let result = (|| -> Result<Measured> {
        let root_uri = Url::from_directory_path(work).map_err(|()| anyhow::anyhow!("a path"))?;
        let uri = Url::from_file_path(&path).map_err(|()| anyhow::anyhow!("a path"))?;
        client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": nomi_testkit::lsp::client_capabilities(),
                "initializationOptions": client.settings,
                "workspaceFolders": [{ "uri": root_uri, "name": "bench" }],
            }),
        )?;
        client.notify("initialized", json!({}));
        client.notify(
            "workspace/didChangeConfiguration",
            json!({ "settings": { "elmLS": client.settings } }),
        );
        // Started: once it has gone quiet, as elm-language-server takes
        // no notice of a file opened while it reads the project.
        client.settle()?;
        // First diagnostics: the error, reported on open.
        let start = Instant::now();
        client.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "elm", "version": 1, "text": broken } }),
        );
        client.diagnostics(&uri, true)?;
        let first = Cost {
            wall: start.elapsed(),
            ..usage(pid)?
        };
        // Saves, fixed and broken in turn.
        let mut version = 1;
        let mut best = Duration::MAX;
        let before = usage(pid)?;
        for i in 0..SAVES {
            let fixing = i % 2 == 0;
            let text = if fixing { &clean } else { &broken };
            version += 1;
            let start = Instant::now();
            client.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [{ "text": text }],
                }),
            );
            fs::write(&path, text)?;
            client.notify(
                "textDocument/didSave",
                json!({ "textDocument": { "uri": uri } }),
            );
            client.diagnostics(&uri, !fixing)?;
            best = best.min(start.elapsed());
        }
        let after = usage(pid)?;
        let save = Cost {
            wall: best,
            cpu: (after.cpu - before.cpu) / SAVES as u32,
            memory: after.memory,
        };
        // Completions, at names in the module's bodies, with it fixed.
        let positions = completion_positions(&clean);
        ensure!(!positions.is_empty(), "nowhere to complete in {entry}");
        let mut best = Duration::MAX;
        let before = usage(pid)?;
        for (line, character) in positions.iter().cycle().take(COMPLETIONS) {
            let start = Instant::now();
            client.request(
                "textDocument/completion",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character },
                }),
            )?;
            best = best.min(start.elapsed());
        }
        let after = usage(pid)?;
        let completion = Cost {
            wall: best,
            cpu: (after.cpu - before.cpu) / COMPLETIONS as u32,
            memory: after.memory,
        };
        let _ = client.request("shutdown", Value::Null);
        client.notify("exit", Value::Null);
        Ok(Measured {
            first,
            save,
            completion,
        })
    })();
    finish(child);
    fs::write(&path, &clean)?;
    result
}

fn finish(child: Child) {
    nomi_testkit::lsp::reap(child);
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

/// The server's CPU time and peak resident memory so far: its process's
/// and its descendants', as a version manager's `node` runs the real one
/// as its child. CPU time counts children that have exited too.
fn usage(pid: u32) -> Result<Cost> {
    let mut tree = vec![pid];
    let mut i = 0;
    let all: Vec<(u32, u32)> = fs::read_dir("/proc")?
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|p| Some((p, stat(p)?.0)))
        .collect();
    while i < tree.len() {
        let parent = tree[i];
        tree.extend(all.iter().filter(|(_, pp)| *pp == parent).map(|(p, _)| *p));
        i += 1;
    }
    let (mut ticks, mut memory) = (0, 0);
    for pid in tree {
        if let Some((_, t)) = stat(pid) {
            ticks += t;
        }
        let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
        memory += status
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .unwrap_or(0);
    }
    let hz = 100;
    Ok(Cost {
        wall: Duration::ZERO,
        cpu: Duration::from_millis(ticks * 1000 / hz),
        memory,
    })
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
    next: i32,
}

impl Client {
    fn notify(&self, method: &str, params: Value) {
        let _ = self
            .connection
            .sender
            .send(Message::Notification(Notification::new(
                method.to_owned(),
                params,
            )));
    }

    /// Sends a request and waits for its answer, answering the server's
    /// own requests meanwhile.
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = RequestId::from(self.next);
        self.connection.sender.send(Message::Request(Request::new(
            id.clone(),
            method.to_owned(),
            params,
        )))?;
        let deadline = Instant::now() + LIMIT;
        loop {
            match self.receive(deadline)? {
                Message::Response(response) if response.id == id => {
                    return Ok(response.result.unwrap_or(Value::Null));
                }
                _ => {}
            }
        }
    }

    /// Waits until diagnostics for `uri` are published with errors, or
    /// with none where `errors` is false.
    fn diagnostics(&mut self, uri: &Url, errors: bool) -> Result<()> {
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Message::Notification(n) = self.receive(deadline)?
                && n.method == "textDocument/publishDiagnostics"
                && n.params["uri"].as_str() == Some(uri.as_str())
            {
                let found = (n.params["diagnostics"].as_array().into_iter().flatten())
                    .any(|d| d["severity"] == 1);
                if found == errors {
                    return Ok(());
                }
            }
        }
    }

    /// Until the server has said nothing for a while.
    fn settle(&mut self) -> Result<()> {
        let deadline = Instant::now() + LIMIT;
        loop {
            match self.receive(Instant::now() + Duration::from_millis(1500)) {
                Ok(_) => {}
                Err(_) if Instant::now() < deadline => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    /// The next message from the server, answering its requests.
    fn receive(&mut self, deadline: Instant) -> Result<Message> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = match self.connection.receiver.recv_timeout(left) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => bail!("the server took too long"),
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
            return Ok(message);
        }
    }
}
