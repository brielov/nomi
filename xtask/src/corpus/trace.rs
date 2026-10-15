//! Editor traces: sessions recorded rather than written.
//!
//! `cargo xtask oracle editor-trace APP` has Helix edit a copy of a
//! corpus app with elm-language-server, driven through tmux by a script
//! of keys, and turns the messages Helix sends, as its log has them, into
//! a session in `corpus/lsp/traces/APP.json`. `cargo xtask corpus lsp`
//! replays it as one more project, `trace/APP`, recorded and compared as
//! the generated sessions are.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use nomi_testkit::lsp::{Method, Session, Step, dir_uri, percent_decoded};
use serde_json::{Map, Value, json};

use crate::oracle::lsp::Tools;
use crate::oracle::which;
use crate::references::References;

/// Where the traces are kept.
pub(crate) const DIR: &str = "corpus/lsp/traces";

/// A key, or keys, to give Helix.
enum Key {
    /// Typed as it is, at once.
    Text(&'static str),
    /// Typed a character at a time, as completion pops up between them.
    Typed(&'static str),
    /// A key tmux names, such as `Escape` or `C-o`.
    Press(&'static str),
    /// A search for the text, which selects it.
    Search(&'static str),
    /// Until the server goes quiet.
    Settle,
}

use Key::{Press, Search, Settle, Text, Typed};

/// What Helix does to an app: the file it opens, and the keys.
struct Script {
    app: &'static str,
    file: &'static str,
    keys: fn() -> Vec<Key>,
}

/// Looking at a name: hover, its definition and back, its references.
fn look(name: &'static str) -> Vec<Key> {
    vec![
        Search(name),
        Settle,
        Text(" k"),
        Settle,
        Press("Escape"),
        Text("gd"),
        Settle,
        Press("C-o"),
        Text("gr"),
        Settle,
        Press("Escape"),
    ]
}

/// The symbols, three names looked at, then a function added above the
/// declaration `before`, typed so completion pops up as it goes, saved
/// and looked at.
fn edit(
    names: [&'static str; 3],
    before: &'static str,
    head: &'static str,
    body: &'static str,
) -> Vec<Key> {
    let mut keys = vec![Settle, Text(" s"), Settle, Press("Escape")];
    for name in names {
        keys.extend(look(name));
    }
    let new_name = head.split(' ').next().unwrap_or(head);
    keys.extend([
        Search(before),
        Text("O"),
        Typed(head),
        // Out of insert mode, which closes completion, and into a new
        // line, which Helix leaves unindented after a declaration's head.
        Press("Escape"),
        Text("o"),
        Text("    "),
        Typed(body),
        Press("Escape"),
        Press("Escape"),
        Text(":w"),
        Press("Enter"),
        Settle,
        // The new function, looked at.
        Search(new_name),
        Settle,
        Text(" k"),
        Settle,
        Press("Escape"),
        Text("gr"),
        Settle,
        Press("Escape"),
    ]);
    keys
}

const SCRIPTS: &[Script] = &[
    Script {
        app: "todomvc",
        file: "src/Main.elm",
        keys: || {
            edit(
                ["updateWithStorage", "viewEntries model", "onEnter"],
                "^infoFooter :",
                "countLeft model =",
                "List.length (List.filter (not << .completed) model.entries)",
            )
        },
    },
    Script {
        app: "elm-spa-example",
        file: "src/Page/Article.elm",
        keys: || {
            edit(
                ["viewButtons", "Article.body", "Api.delete"],
                "^viewButtons :",
                "titleOf article =",
                "Html.text (Article.metadata article).title",
            )
        },
    },
];

/// `cargo xtask oracle editor-trace`.
pub(crate) fn record(root: &Path, refs: &References, app: &str) -> Result<()> {
    let script = SCRIPTS
        .iter()
        .find(|s| s.app == app)
        .with_context(|| format!("no script for `{app}`; there are {}", names()))?;
    let project = super::lsp::app(root, app)?;
    let scratch = super::lsp::sandbox()?.join("traces");
    let work = scratch.join(app);
    super::lsp::copy(&project, &work)?;
    let work = fs::canonicalize(&work)?;
    let tools = super::lsp::server(refs, &super::lsp::sandbox()?)?;
    let config = scratch.join(format!("{app}-config"));
    write_config(&config, &tools, &scratch.join("no-elm-review"))?;
    let log = scratch.join(format!("{app}.log"));
    let _ = fs::remove_file(&log);
    drive(&work, &config, &log, script)?;
    let text = fs::read_to_string(&log).context("Helix's log")?;
    let title = format!("Helix editing {} in {}", script.file, script.app);
    let session = session(&text, &dir_uri(&work), title)?;
    let out = root.join(DIR);
    fs::create_dir_all(&out)?;
    let path = out.join(format!("{app}.json"));
    let steps = session.requests.len();
    // Through a `Value`, whose keys are sorted, as the traces are written.
    let session = serde_json::to_value(session)?;
    fs::write(&path, nomi_testkit::lsp::to_text(&session))?;
    eprintln!("{}: {steps} steps", path.display());
    Ok(())
}

fn names() -> String {
    SCRIPTS.iter().map(|s| s.app).collect::<Vec<_>>().join(", ")
}

/// Helix's configuration: elm-language-server for Elm, with the
/// recordings' settings, and completion that pops up soon.
fn write_config(dir: &Path, tools: &Tools, no_review: &Path) -> Result<()> {
    let helix = dir.join("helix");
    fs::create_dir_all(&helix)?;
    let mut settings = nomi_testkit::lsp::default_settings();
    settings.extend(tools.settings(no_review));
    let server = json!({
        "command": tools.node.to_string_lossy(),
        "args": [tools.server.to_string_lossy(), "--stdio"],
        "environment": {
            "ELM_HOME": tools.home.to_string_lossy(),
            "PATH": tools.path.to_string_lossy(),
        },
        "config": settings,
    });
    fs::write(helix.join("languages.toml"), languages(&server)?)?;
    fs::write(
        helix.join("config.toml"),
        "[editor]\nidle-timeout = 200\ncompletion-timeout = 200\nauto-pairs = false\n",
    )?;
    Ok(())
}

/// Helix's `languages.toml`: Elm, served by `server`.
fn languages(server: &Value) -> Result<String> {
    let languages = json!({
        "language-server": { "elmls": server },
        "language": [{ "name": "elm", "language-servers": ["elmls"], "auto-format": false }],
    });
    Ok(toml::to_string(&languages)?)
}

/// Runs Helix on `script.file` in a tmux session and gives it the keys.
fn drive(work: &Path, config: &Path, log: &Path, script: &Script) -> Result<()> {
    let session = format!("nomi-trace-{}", std::process::id());
    let hx = which("hx").context("Helix (`hx`) isn't on PATH")?;
    let command = format!(
        "XDG_CONFIG_HOME={} {} -vvv --log {} {}",
        shell(&config.to_string_lossy()),
        shell(&hx.to_string_lossy()),
        shell(&log.to_string_lossy()),
        script.file,
    );
    // A tmux server of its own, which reads an `Escape` as itself however
    // soon a key follows it, not as Alt with that key.
    let status = tmux()
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-x",
            "160",
            "-y",
            "50",
            "-c",
        ])
        .arg(work)
        .arg(&command)
        .status()
        .context("starting tmux")?;
    ensure!(status.success(), "tmux failed to start Helix");
    tmux().args(["set", "-s", "escape-time", "0"]).status()?;
    let result = (|| -> Result<()> {
        // The server publishes nothing on open for a project that
        // compiles: it's ready once it has gone quiet after the file opens.
        wait_for(log, "textDocument/didOpen", Duration::from_secs(120))?;
        settle(log);
        for key in (script.keys)() {
            match key {
                Text(text) => send(&session, &["-l", text])?,
                Search(text) => {
                    send(&session, &["-l", &format!("/{text}")])?;
                    send(&session, &["Enter"])?;
                }
                Typed(text) => {
                    for c in text.chars() {
                        send(&session, &["-l", &c.to_string()])?;
                        std::thread::sleep(Duration::from_millis(350));
                    }
                }
                Press(name) => send(&session, &[name])?,
                Settle => settle(log),
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        send(&session, &["Escape"])?;
        send(&session, &["-l", ":q!"])?;
        send(&session, &["Enter"])?;
        settle(log);
        Ok(())
    })();
    let _ = tmux().args(["kill-server"]).status();
    result
}

/// tmux, on the traces' own server.
fn tmux() -> Command {
    let mut command = Command::new("tmux");
    command.args(["-L", "nomi-trace"]);
    command
}

fn send(session: &str, args: &[&str]) -> Result<()> {
    let status = tmux()
        .args(["send-keys", "-t", session])
        .args(args)
        .status()?;
    ensure!(status.success(), "tmux send-keys failed");
    Ok(())
}

/// Until `log` mentions `text`.
fn wait_for(log: &Path, text: &str, limit: Duration) -> Result<()> {
    let start = Instant::now();
    while !fs::read_to_string(log).is_ok_and(|t| t.contains(text)) {
        if start.elapsed() > limit {
            bail!("Helix's log never mentioned {text}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Until `log` stops growing for a while.
fn settle(log: &Path) {
    let quiet = Duration::from_millis(1500);
    let start = Instant::now();
    let mut size = fs::metadata(log).map_or(0, |m| m.len());
    let mut since = Instant::now();
    while since.elapsed() < quiet {
        std::thread::sleep(Duration::from_millis(100));
        let now = fs::metadata(log).map_or(0, |m| m.len());
        if now != size {
            size = now;
            since = Instant::now();
        }
        if start.elapsed() > Duration::from_secs(60) {
            break;
        }
    }
}

fn shell(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The session Helix's messages to the server make, as Helix's log has
/// them: its documents' opens, changes, saves and closes, and its
/// requests about them, by path in the project at `root`, a URI.
fn session(log: &str, root: &str, title: String) -> Result<Session> {
    let mut open = Vec::new();
    let mut requests = Vec::new();
    for line in log.lines() {
        let Some((_, message)) = line.split_once(" -> ") else {
            continue;
        };
        if !line.contains("helix_lsp::transport") {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(message) else {
            continue;
        };
        let Some(method) = message["method"].as_str() else {
            // A reply to the server's own request.
            continue;
        };
        let mut params = message["params"].as_object().cloned().unwrap_or_default();
        let file = (params.remove("textDocument"))
            .and_then(|d| d["uri"].as_str().map(str::to_owned))
            .and_then(|uri| file_of(&uri, root));
        let Some(file) = file else {
            continue;
        };
        let method = Method::from(method.to_owned());
        match method {
            // The first file, opened before anything is asked.
            Method::DidOpen if requests.is_empty() && open.is_empty() => {
                open.push(file);
            }
            Method::DidOpen | Method::DidSave | Method::DidClose => {
                requests.push(Step::on(method, file));
            }
            Method::DidChange => {
                let changes = params.remove("contentChanges").unwrap_or(json!([]));
                requests.push(Step {
                    params: Map::from_iter([("contentChanges".to_owned(), changes)]),
                    ..Step::on(method, file)
                });
            }
            _ if message.get("id").is_some() => {
                let at = (params.remove("position"))
                    .or_else(|| params.remove("range"))
                    .map(serde_json::from_value)
                    .transpose()
                    .context("a position or range Helix sent")?;
                // The driver gives a code action the diagnostics it has.
                if method == Method::CodeAction {
                    params.remove("context");
                }
                // Progress tokens are Helix's own.
                params.remove("workDoneToken");
                params.remove("partialResultToken");
                requests.push(Step {
                    at,
                    params,
                    ..Step::on(method, file)
                });
            }
            _ => {}
        }
    }
    ensure!(!open.is_empty(), "Helix opened nothing");
    Ok(Session {
        title: Some(title),
        open,
        requests,
        ..Session::default()
    })
}

/// The path in the project of `uri`, under `root`.
fn file_of(uri: &str, root: &str) -> Option<String> {
    let rest = uri.strip_prefix(root)?.strip_prefix('/')?;
    Some(percent_decoded(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helix_reads_the_server_and_its_settings() {
        let server = json!({
            "command": "/bin/node",
            "args": ["/srv/\"index\".js", "--stdio"],
            "environment": { "ELM_HOME": "/home", "PATH": "/bin:/usr/bin" },
            "config": { "trace": { "server": "off" }, "elmPath": "/bin/elm" },
        });
        let read: toml::Table = toml::from_str(&languages(&server).unwrap()).unwrap();
        let expected: toml::Table = toml::from_str(concat!(
            "[language-server.elmls]\n",
            "command = \"/bin/node\"\n",
            "args = [\"/srv/\\\"index\\\".js\", \"--stdio\"]\n",
            "environment = { ELM_HOME = \"/home\", PATH = \"/bin:/usr/bin\" }\n",
            "config = { trace = { server = \"off\" }, elmPath = \"/bin/elm\" }\n",
            "[[language]]\n",
            "name = \"elm\"\n",
            "language-servers = [\"elmls\"]\n",
            "auto-format = false\n",
        ))
        .unwrap();
        assert_eq!(read, expected);
    }

    #[test]
    fn helix_messages_make_a_session() {
        let sent = |message: Value| format!("2026 helix_lsp::transport [INFO] elmls -> {message}");
        let doc = |file: &str| json!({ "uri": format!("file:///p/{file}") });
        let elsewhere = json!({ "uri": "file:///q/B.elm" });
        let at = json!({ "line": 1, "character": 2 });
        let range = json!({ "start": at, "end": { "line": 1, "character": 5 } });
        let log = [
            sent(
                json!({ "method": "textDocument/didOpen", "params": { "textDocument": doc("src/A.elm") } }),
            ),
            sent(
                json!({ "method": "textDocument/didOpen", "params": { "textDocument": elsewhere } }),
            ),
            sent(json!({ "method": "textDocument/didChange", "params": {
                "textDocument": doc("src/A.elm"), "contentChanges": [{ "text": "x" }], "other": 1,
            } })),
            sent(json!({ "id": 1, "method": "textDocument/hover", "params": {
                "textDocument": doc("src/A.elm"), "position": at, "workDoneToken": "t",
            } })),
            sent(
                json!({ "id": 2, "method": "textDocument/codeAction", "params": {
                "textDocument": doc("src/A.elm"), "range": range, "context": {}, "partialResultToken": "t",
            } }),
            ),
            sent(json!({ "id": 3, "result": null })),
            "2026 helix_term [INFO] -> not a message".to_owned(),
        ];
        let session = session(&log.join("\n"), "file:///p", "a title".to_owned()).unwrap();
        assert_eq!(
            serde_json::to_value(session).unwrap(),
            json!({
                "title": "a title",
                "open": ["src/A.elm"],
                "requests": [
                    { "method": "textDocument/didChange", "file": "src/A.elm",
                      "params": { "contentChanges": [{ "text": "x" }] } },
                    { "method": "textDocument/hover", "file": "src/A.elm", "at": at },
                    { "method": "textDocument/codeAction", "file": "src/A.elm", "at": range },
                ],
            })
        );
    }
}
