//! elm-format's command line, on what its transcripts can't hold: bytes
//! that aren't UTF-8, and a console that fails. The CLI's transcript test
//! covers the rest against elm-format itself.

use std::fs;
use std::io;
use std::path::PathBuf;

use nomi_engine::Vfs;
use nomi_engine::console::Captured;
use nomi_engine::elm_format::{ElmVersion, Exit, Options, run};
use nomi_engine::format::decode;

fn stdin(validate: bool) -> Options {
    Options {
        stdin: true,
        validate,
        elm_version: Some(ElmVersion::V0_19),
        ..Options::default()
    }
}

#[test]
fn text_that_is_not_utf8_fails_where_it_stops_being_utf8() {
    assert_eq!(decode(b"ab".to_vec()).unwrap(), "ab");
    let at = decode(b"module A\n  \xe9 \xff".to_vec()).unwrap_err();
    assert_eq!((at.row, at.column), (2, 3));
    let at = decode("é\u{1F600}".bytes().chain([0xff]).collect()).unwrap_err();
    assert_eq!((at.row, at.column), (1, 3));
}

#[test]
fn standard_input_that_is_not_utf8_is_a_syntax_error() {
    let mut console = Captured {
        input: b"module A exposing (..)\nx = \"\xff\"\n".to_vec(),
        ..Captured::default()
    };
    assert_eq!(run(&Vfs::new(), &stdin(false), &mut console), Exit::Failure);
    assert_eq!(console.out, "");
    assert_eq!(
        console.err,
        "Unable to parse file <STDIN>:2:6 To see a detailed explanation, run elm make on the file.\n"
    );
    let mut console = Captured {
        input: b"\xff".to_vec(),
        ..Captured::default()
    };
    assert_eq!(run(&Vfs::new(), &stdin(true), &mut console), Exit::Failure);
    assert_eq!(
        console.out,
        "[{\"path\":\"<STDIN>\",\"message\":\"Error parsing the file\"}\n]\n"
    );
}

#[test]
fn a_module_that_is_not_utf8_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    let bytes = b"module A exposing (..)\nx  =  \"\xff\"\n";
    fs::write(&path, bytes).unwrap();
    let options = Options {
        inputs: vec![path.clone()],
        yes: true,
        elm_version: Some(ElmVersion::V0_19),
        ..Options::default()
    };
    let mut console = Captured::default();
    assert_eq!(run(&Vfs::new(), &options, &mut console), Exit::Failure);
    assert_eq!(
        console.out,
        format!(
            "Processing file {0}\nUnable to parse file {0}:2:8 To see a detailed explanation, run elm make on the file.\n",
            path.display()
        )
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn a_failed_answer_stops_before_any_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("A.elm");
    fs::write(&path, "module A exposing (..)\nx  =  1\n").unwrap();
    let options = Options {
        inputs: vec![PathBuf::from(&path)],
        elm_version: Some(ElmVersion::V0_19),
        ..Options::default()
    };
    let mut console = Captured {
        lines: [
            Ok("maybe".to_owned()),
            Err(io::Error::other("the terminal went away")),
        ]
        .into(),
        ..Captured::default()
    };
    assert_eq!(run(&Vfs::new(), &options, &mut console), Exit::Failure);
    assert!(
        console
            .out
            .ends_with("(y/n)\n\nMust type 'y' for yes or 'n' for no: ")
    );
    assert_eq!(console.err, "elm-format: <stdin>: the terminal went away\n");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "module A exposing (..)\nx  =  1\n"
    );
}
