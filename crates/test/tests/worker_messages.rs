//! A worker reads the supervisor's messages by lines, however the socket
//! splits or joins them.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// A test program that reports it finished, then prints the type of
/// the message it is sent and exits.
const PROGRAM: &str = "exports.run = function (index, receive) {
  receive({ type: 'FINISHED', results: {} });
  return function (message) { console.log(message.type); process.exit(0); };
};
";

#[test]
fn a_message_split_across_writes_arrives_whole() {
    nomi_testkit::require_node();
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program.js");
    std::fs::write(&program, PROGRAM).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = Command::new("node")
        .args(["-e", include_str!("../runner/driver.js")])
        .arg(&program)
        .args(["0", &port.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (mut socket, _) = listener.accept().unwrap();
    // The worker's FINISHED, then the summary in two writes.
    let mut first = [0; 1];
    socket.read_exact(&mut first).unwrap();
    let summary = "{\"type\":\"SUMMARY\",\"todos\":[\"a summary sent in two pieces\"]}\n";
    let (head, tail) = summary.split_at(20);
    socket.write_all(head.as_bytes()).unwrap();
    socket.flush().unwrap();
    thread::sleep(Duration::from_millis(50));
    socket.write_all(tail.as_bytes()).unwrap();
    let output = worker.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "SUMMARY\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
}
