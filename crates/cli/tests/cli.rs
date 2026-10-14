//! `nomi`'s own command line: its version, an unknown argument, and the
//! language server on the standard streams.

use std::process::Command;

#[test]
fn prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_nomi"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("nomi {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn rejects_unknown_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_nomi"))
        .arg("--bogus")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

/// `nomi lsp` speaks the protocol on standard input and output: a client
/// that initializes and shuts it down gets its answers, and it exits
/// cleanly.
#[test]
fn lsp_serves_standard_streams() {
    use std::io::Write;
    use std::process::Stdio;

    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{body}", body.len());
    let mut child = Command::new(env!("CARGO_BIN_EXE_nomi"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ] {
        stdin.write_all(frame(body).as_bytes()).unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(r#""positionEncoding":"utf-16""#),
        "{stdout}"
    );
    assert!(stdout.contains(r#""id":2"#), "{stdout}");
}
