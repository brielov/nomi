//! nomi invoked as `elm`: the Elm compiler's commands, as they come.
//! What they do is `nomi_engine`'s; commands later milestones implement
//! say they aren't there yet.

use std::ffi::OsString;
use std::process::ExitCode;

use nomi_engine::console::Personality;

pub fn main(args: impl Iterator<Item = OsString>) -> ExitCode {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let succeeded = match args[..] {
        ["init"] => crate::new(Personality::Elm, None),
        ["install"] => crate::add(Personality::Elm, None),
        ["install", package] => crate::add(Personality::Elm, Some(package)),
        _ => {
            eprintln!("error: nomi doesn't answer to `elm {}` yet", args.join(" "));
            false
        }
    };
    crate::exit(succeeded)
}
