//! `nomi lsp` installing packages, as a code action's command asks it
//! to: each question about an install is answered for its own, and the
//! next request sees what the install wrote.
//!
//! The server runs offline, with a cache serving the packages vendored
//! for the compiler's tests.

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::Url;
use nomi_testkit::lsp::{child_connection, reap};
use nomi_testkit::packages::CORE_AND_JSON_APP;
use serde_json::{Value, json};
use website::{fixture_cache, offline};

/// A module of a package the project doesn't depend on yet.
const MAIN: &str = "module Main exposing (main)\n\nimport Html\n\n\nmain =\n    Html.text \"hi\"\n";

/// The editor's side of a `nomi lsp` session.
struct Client {
    child: Child,
    connection: Connection,
    next_id: i32,
    /// The questions the server asked, not yet answered.
    questions: Vec<Request>,
    /// The messages the server showed.
    shown: Vec<String>,
}

impl Client {
    /// A server started offline with the cache at `cache`, and
    /// initialized.
    fn start(cache: &Path) -> Client {
        let mut child = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache)
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let connection =
            child_connection(child.stdin.take().unwrap(), child.stdout.take().unwrap());
        let mut client = Client {
            child,
            connection,
            next_id: 0,
            shown: Vec::new(),
            questions: Vec::new(),
        };
        client.request(
            "initialize",
            json!({ "processId": null, "capabilities": {} }),
        );
        client.notify("initialized", json!({}));
        client
    }

    /// Sends a request and waits for its result, keeping the questions
    /// asked meanwhile.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let request = Request::new(id.clone(), method.to_owned(), params);
        self.connection
            .sender
            .send(Message::Request(request))
            .unwrap();
        loop {
            let message = (self.connection.receiver)
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|e| panic!("no answer to {method}: {e}"));
            match message {
                Message::Response(response) if response.id == id => {
                    assert!(response.error.is_none(), "{response:?}");
                    return response.result.unwrap_or(Value::Null);
                }
                Message::Request(question) => self.questions.push(question),
                Message::Notification(note) if note.method == "window/showMessage" => {
                    let message = note.params["message"].as_str().unwrap_or_default();
                    self.shown.push(message.to_owned());
                }
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

    /// Answers `question` with the action titled `title`.
    fn answer(&self, question: &Request, title: &str) {
        let response = Response::new_ok(question.id.clone(), json!({ "title": title }));
        self.connection
            .sender
            .send(Message::Response(response))
            .unwrap();
    }

    /// Asks to install elm/html in the project of the file at `uri`.
    fn install_html(&mut self, uri: &str) {
        let command = json!({ "command": "nomi.installPackage", "arguments": [uri, "elm/html"] });
        self.request("workspace/executeCommand", command);
    }

    fn shutdown(mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        reap(self.child);
    }
}

/// An application of elm/core and elm/json at `dir`, and the URI of its
/// `src/Main.elm`, which imports `Html`.
fn project(dir: &Path) -> String {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("elm.json"), CORE_AND_JSON_APP).unwrap();
    fs::write(dir.join("src/Main.elm"), MAIN).unwrap();
    Url::from_file_path(dir.join("src/Main.elm"))
        .unwrap()
        .to_string()
}

/// Two projects asked about installing the same package, answered in
/// the other order: each answer installs, or doesn't, in its own.
#[test]
fn each_install_question_answers_for_its_own_project() {
    let cache = fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(work.path()).unwrap();
    let mut client = Client::start(cache.path());
    client.install_html(&project(&root.join("first")));
    client.install_html(&project(&root.join("second")));
    let [first, second] = <[Request; 2]>::try_from(std::mem::take(&mut client.questions)).unwrap();
    client.answer(&second, "No");
    client.answer(&first, "Yes");
    // The server reads its messages in turn: once it answers this, it's
    // done with both answers.
    client.shutdown();
    let elm_json = |dir: &str| fs::read_to_string(root.join(dir).join("elm.json")).unwrap();
    assert!(
        elm_json("first").contains("\"elm/html\""),
        "{}",
        elm_json("first")
    );
    assert_eq!(elm_json("second"), CORE_AND_JSON_APP);
}

/// A request right after an install, with no edit between, sees the
/// package installed.
#[test]
fn a_request_after_an_install_sees_the_package() {
    let cache = fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let main = project(&nomi_engine::canonical(work.path()).unwrap());
    let mut client = Client::start(cache.path());
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": main, "languageId": "elm", "version": 1, "text": MAIN } }),
    );
    // `text` in `Html.text`.
    let hover =
        json!({ "textDocument": { "uri": main }, "position": { "line": 6, "character": 10 } });
    assert_eq!(
        client.request("textDocument/hover", hover.clone()),
        Value::Null
    );
    client.install_html(&main);
    let question = client.questions.pop().unwrap();
    client.answer(&question, "Yes");
    let found = client.request("textDocument/hover", hover);
    let shown = found["contents"]["value"].as_str().unwrap_or_default();
    assert!(shown.contains("text : String -> Html.Html msg"), "{found}");
    client.shutdown();
}

/// The question shows the install's prompt alone, and what the editor
/// shows once it is answered, the answered run's words alone.
#[test]
fn an_install_shows_its_question_and_then_its_outcome() {
    let cache = fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let main = project(&nomi_engine::canonical(work.path()).unwrap());
    let mut client = Client::start(cache.path());
    client.install_html(&main);
    let question = client.questions.pop().unwrap();
    let asked = question.params["message"].as_str().unwrap();
    assert!(
        asked.ends_with("Would you like me to update your elm.json accordingly?"),
        "{asked}"
    );
    client.answer(&question, "Yes");
    // Its answer is read before the next request is.
    client.request("shutdown", Value::Null);
    let shown = std::mem::take(&mut client.shown);
    client.notify("exit", Value::Null);
    reap(client.child);
    assert_eq!(shown, ["Success!"]);
}
