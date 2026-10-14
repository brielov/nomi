//! `nomi lsp` in conversation with a scripted client: the server runs on
//! an in-memory connection, and each test sends what an editor sends and
//! checks what comes back.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::Url;
use serde_json::{Value, json};

const APP: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": { "direct": { "elm/core": "1.0.5", "elm/json": "1.1.4" }, "indirect": {} },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

struct Client {
    connection: Connection,
    server: Option<thread::JoinHandle<Result<(), nomi_lsp::Error>>>,
    next_id: i32,
    capabilities: Value,
}

impl Client {
    /// A server started and initialized for a client that offers `encodings`.
    fn start(encodings: &[&str]) -> Client {
        Client::start_with(encodings, &Value::Null)
    }

    /// [`Client::start`], with `options` as the initialization options.
    fn start_with(encodings: &[&str], options: &Value) -> Client {
        let (server, connection) = Connection::memory();
        let handle = thread::spawn(move || nomi_lsp::run(&server));
        let mut client = Client {
            connection,
            server: Some(handle),
            next_id: 0,
            capabilities: Value::Null,
        };
        let response = client.request(
            "initialize",
            json!({
                "processId": null,
                "capabilities": { "general": { "positionEncodings": encodings } },
                "initializationOptions": options,
            }),
        );
        client.capabilities = response.result.unwrap()["capabilities"].clone();
        client.notify("initialized", json!({}));
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Response {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let request = Request::new(id.clone(), method.to_owned(), params);
        self.connection
            .sender
            .send(Message::Request(request))
            .unwrap();
        loop {
            match self.receive() {
                Message::Response(response) if response.id == id => return response,
                _ => {}
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        let notification = Notification::new(method.to_owned(), params);
        self.connection
            .sender
            .send(Message::Notification(notification))
            .unwrap();
    }

    fn receive(&self) -> Message {
        self.connection
            .receiver
            .recv_timeout(Duration::from_secs(20))
            .expect("the server answers")
    }

    /// The diagnostics published for `uri` once they satisfy `settled`:
    /// a change makes every open document's diagnostics published again,
    /// so earlier ones may come first.
    fn diagnostics(&self, uri: &Url, settled: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Message::Notification(n) = self.receive()
                && n.method == "textDocument/publishDiagnostics"
                && n.params["uri"] == uri.as_str()
            {
                let found = n.params["diagnostics"].as_array().unwrap().clone();
                if settled(&found) {
                    return found;
                }
            }
        }
        panic!("no settled diagnostics for {uri}");
    }

    fn open(&self, uri: &Url, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": uri, "languageId": "elm", "version": 1, "text": text,
            } }),
        );
    }

    fn shutdown(mut self) {
        let response = self.request("shutdown", Value::Null);
        assert!(response.error.is_none(), "{response:?}");
        self.notify("exit", Value::Null);
        let server = self.server.take().unwrap();
        server.join().unwrap().unwrap();
    }
}

/// A project directory with `elm.json` and `src/`, and the URI of its
/// `src/Main.elm`.
fn project() -> (tempfile::TempDir, Url) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(root(&dir).join("elm.json"), APP).unwrap();
    fs::create_dir(root(&dir).join("src")).unwrap();
    let uri = uri(&root(&dir).join("src/Main.elm"));
    (dir, uri)
}

/// The project's directory by its canonical name, which the server gives
/// the files it finds on disk; on macOS the temporary directory is behind
/// a link.
fn root(dir: &tempfile::TempDir) -> std::path::PathBuf {
    nomi_engine::canonical(dir.path()).unwrap()
}

fn uri(path: &Path) -> Url {
    Url::from_file_path(path).unwrap()
}

/// `Main`, which uses `Other.helper`.
const USES_OTHER: &str =
    "module Main exposing (main)\n\nimport Other\n\n\nmain =\n    Other.helper ()\n";

/// A project whose `src/Other.elm` is saved, not open, and the URIs of
/// its `src/Main.elm` and `src/Other.elm`.
fn two_modules() -> (tempfile::TempDir, Url, Url) {
    let (dir, main) = project();
    let other = root(&dir).join("src/Other.elm");
    fs::write(
        &other,
        "module Other exposing (helper)\n\n\nhelper x =\n    x\n",
    )
    .unwrap();
    (dir, main, uri(&other))
}

/// The parameters of a request at `line` and `character` in `uri`.
fn at(uri: &Url, line: u32, character: u32) -> Value {
    json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } })
}

#[test]
fn the_handshake_offers_the_capabilities_and_negotiates_the_encoding() {
    let client = Client::start(&["utf-16"]);
    let capabilities = &client.capabilities;
    assert_eq!(capabilities["positionEncoding"], "utf-16");
    assert_eq!(
        capabilities["textDocumentSync"],
        json!({ "openClose": true, "change": 2, "save": true })
    );
    assert_eq!(capabilities["documentFormattingProvider"], true);
    assert_eq!(capabilities["documentSymbolProvider"], true);
    assert_eq!(capabilities["definitionProvider"], true);
    assert_eq!(capabilities["referencesProvider"], true);
    assert_eq!(capabilities["documentHighlightProvider"], true);
    assert_eq!(capabilities["hoverProvider"], true);
    assert_eq!(
        capabilities["completionProvider"]["triggerCharacters"],
        json!(["."])
    );
    client.shutdown();
    let client = Client::start(&["utf-16", "utf-8"]);
    assert_eq!(client.capabilities["positionEncoding"], "utf-8");
    client.shutdown();
}

#[test]
fn syntax_errors_come_and_go_with_the_text() {
    let (_dir, main) = project();
    let client = Client::start(&[]);
    // The emoji takes two UTF-16 units, so the error's column says which
    // unit the server counts in.
    let broken = "module Main exposing (x)\n\n\nx : String\nx =\n    \"😀\" ++ = \"a\"\n";
    client.open(&main, broken);
    let found = client.diagnostics(&main, |d| !d.is_empty());
    assert_eq!(found.len(), 1);
    let message = found[0]["message"].as_str().unwrap();
    // As elm-language-server writes elm's reports: the title, then the
    // message.
    assert!(message.starts_with("MISSING EXPRESSION - "), "{message}");
    assert_eq!(found[0]["severity"], 1);
    assert_eq!(
        found[0]["range"]["start"],
        json!({ "line": 5, "character": 12 })
    );
    // Replace `= "a"` with `"a"`: the error goes, whether or not the
    // machine's package cache lets the types be checked.
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": main, "version": 2 },
            "contentChanges": [{
                "range": { "start": { "line": 5, "character": 12 }, "end": { "line": 5, "character": 14 } },
                "text": "",
            }],
        }),
    );
    client.diagnostics(&main, <[Value]>::is_empty);
    client.shutdown();
}

#[test]
fn diagnostics_wait_for_a_save_when_asked_to() {
    let (_dir, main) = project();
    let client = Client::start_with(&[], &json!({ "onlyUpdateDiagnosticsOnSave": true }));
    client.open(&main, "module Main exposing (x)\n\n\nx =\n    = 1\n");
    assert_eq!(client.diagnostics(&main, |d| !d.is_empty()).len(), 1);
    // Fixed, but not saved: nothing new comes.
    let fixed = "module Main exposing (x)\n\n\nx =\n    1\n";
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": main, "version": 2 },
            "contentChanges": [{ "text": fixed }],
        }),
    );
    let quiet = client
        .connection
        .receiver
        .recv_timeout(Duration::from_millis(600));
    assert!(quiet.is_err(), "{quiet:?}");
    client.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": main } }),
    );
    client.diagnostics(&main, <[Value]>::is_empty);
    client.shutdown();
}

#[test]
fn an_edit_in_one_document_leaves_anothers_diagnostics() {
    let (dir, main) = project();
    let other = uri(&root(&dir).join("src/Other.elm"));
    let client = Client::start(&[]);
    client.open(&main, "module Main exposing (x)\n\n\nx =\n    = 1\n");
    client.open(&other, "module Other exposing (y)\n\n\ny =\n    2\n");
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": other, "version": 2 },
            "contentChanges": [{ "text": "module Other exposing (y)\n\n\ny =\n    3\n" }],
        }),
    );
    let found = client.diagnostics(&main, |d| !d.is_empty());
    assert_eq!(found.len(), 1, "{found:#?}");
    client.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": main } }),
    );
    client.diagnostics(&main, <[Value]>::is_empty);
    client.shutdown();
}

#[test]
fn formatting_answers_with_the_edits_elm_format_makes() {
    let (_dir, main) = project();
    let mut client = Client::start(&[]);
    let text = "module Main exposing (x)\nx = 1\n";
    client.open(&main, text);
    let response = client.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": main }, "options": { "tabSize": 4, "insertSpaces": true } }),
    );
    assert_eq!(
        response.result.unwrap(),
        // `x = 1` becomes `x =\n    1` after two blank lines; ` 1` stays.
        json!([{
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 3 } },
            "newText": "\n\nx =\n   ",
        }]),
    );
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": main, "version": 2 },
            "contentChanges": [{ "text": "x = = 1\n" }],
        }),
    );
    let response = client.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": main }, "options": { "tabSize": 4, "insertSpaces": true } }),
    );
    assert_eq!(response.result.unwrap(), Value::Null);
    client.shutdown();
}

#[test]
fn document_symbols_are_flat_as_elm_language_servers() {
    let (_dir, main) = project();
    let mut client = Client::start(&[]);
    client.open(
        &main,
        "module Main exposing (..)\n\n\ntype Shape\n    = Circle\n\n\narea =\n    1\n",
    );
    let response = client.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": main } }),
    );
    let symbols = response.result.unwrap();
    let listed: Vec<(String, u64)> = symbols
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_owned(),
                s["kind"].as_u64().unwrap(),
            )
        })
        .collect();
    let expected = [("Main", 2), ("Shape", 10), ("Circle", 22), ("area", 12)];
    assert_eq!(
        listed,
        expected
            .map(|(name, kind)| (name.to_owned(), kind))
            .to_vec()
    );
    assert_eq!(
        symbols[3]["location"],
        json!({
            "uri": main,
            "range": { "start": { "line": 7, "character": 0 }, "end": { "line": 8, "character": 5 } },
        }),
    );
    client.shutdown();
}

#[test]
fn what_nomi_doesnt_answer_is_said_so() {
    let mut client = Client::start(&[]);
    // Signature help comes after parity with elm-language-server.
    let response = client.request("textDocument/signatureHelp", json!({}));
    assert_eq!(
        response.error.unwrap().code,
        lsp_server::ErrorCode::MethodNotFound as i32
    );
    client.shutdown();
}

#[test]
fn navigation_goes_to_definitions_and_finds_references_across_modules() {
    // `Other` is saved, not open: navigation reads it from the disk.
    let (_dir, main, other) = two_modules();
    let mut client = Client::start(&[]);
    client.open(&main, USES_OTHER);
    // `helper` in `Other.helper`, to its name and parameters.
    let response = client.request("textDocument/definition", at(&main, 6, 12));
    let found = response.result.unwrap();
    assert_eq!(found["uri"], other.as_str());
    assert_eq!(
        found["range"],
        json!({ "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 8 } })
    );
    let mut params = at(&main, 6, 12);
    params["context"] = json!({ "includeDeclaration": false });
    let response = client.request("textDocument/references", params);
    let found = response.result.unwrap();
    let found: Vec<(&str, &Value)> = (found.as_array().unwrap().iter())
        .map(|l| (l["uri"].as_str().unwrap(), &l["range"]["start"]))
        .collect();
    assert_eq!(
        found,
        [
            (other.as_str(), &json!({ "line": 0, "character": 23 })),
            (main.as_str(), &json!({ "line": 6, "character": 10 })),
        ]
    );
    // `main`: the exposing list, then its definition, in the text's order.
    let response = client.request("textDocument/documentHighlight", at(&main, 5, 1));
    let found = response.result.unwrap();
    let starts: Vec<&Value> = (found.as_array().unwrap().iter())
        .map(|h| &h["range"]["start"])
        .collect();
    assert_eq!(
        starts,
        [
            &json!({ "line": 0, "character": 22 }),
            &json!({ "line": 5, "character": 0 })
        ]
    );
    client.shutdown();
}

#[test]
fn hover_and_completion_answer_without_packages() {
    let (_dir, main, _) = two_modules();
    let mut client = Client::start(&[]);
    client.open(&main, USES_OTHER);
    // `main`'s name: its inferred annotation, then its name.
    let response = client.request("textDocument/hover", at(&main, 5, 1));
    let found = response.result.unwrap();
    assert_eq!(found["contents"]["kind"], "markdown");
    assert_eq!(
        found["contents"]["value"],
        "\n```elm\nmain : ()\n```\n\n\n---\n\n\n```elm\nmain\n```\n"
    );
    // After `Other.`: what Other exposes.
    let response = client.request("textDocument/completion", at(&main, 6, 10));
    let labels: Vec<&str> = (response.result.as_ref().unwrap().as_array().unwrap().iter())
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["helper"]);
    client.shutdown();
}

#[test]
fn a_cancelled_request_is_still_answered() {
    let (_dir, main) = project();
    let mut client = Client::start(&[]);
    client.open(&main, "module Main exposing (x)\n\n\nx : Int\nx =\n    1\n");
    // Asked and cancelled at once: the answer may be the result or the
    // cancellation's error, but one comes, and the server goes on.
    client.next_id += 1;
    let id = RequestId::from(client.next_id);
    let params = json!({ "textDocument": { "uri": main } });
    let request = Request::new(
        id.clone(),
        "textDocument/documentSymbol".to_owned(),
        params.clone(),
    );
    client
        .connection
        .sender
        .send(Message::Request(request))
        .unwrap();
    client.notify("$/cancelRequest", json!({ "id": client.next_id }));
    loop {
        match client.receive() {
            Message::Response(response) if response.id == id => {
                if let Some(error) = &response.error {
                    assert_eq!(error.code, -32800, "{error:?}");
                }
                break;
            }
            _ => {}
        }
    }
    let after = client.request("textDocument/documentSymbol", params);
    assert!(after.error.is_none(), "{after:?}");
    client.shutdown();
}
