//! Running an upstream tool within a time limit, with its output written
//! to files.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result};
use nomi_testkit::process::run_within;

/// [`run_within`] with the command's output in `.stdout` and `.stderr`
/// in `dir`, read back once it has ended, for a tool whose output can be
/// large and that needs no input.
pub(crate) fn run_to_files(
    command: &mut Command,
    dir: &Path,
    limit: Duration,
) -> Result<Option<Output>> {
    let (stdout, stderr) = (dir.join(".stdout"), dir.join(".stderr"));
    command
        .stdout(fs::File::create(&stdout)?)
        .stderr(fs::File::create(&stderr)?);
    let program = command.get_program().to_string_lossy().into_owned();
    let Ok(ended) = run_within(command, &[], Some(limit)).with_context(|| program)? else {
        return Ok(None);
    };
    Ok(Some(Output {
        status: ended.status,
        stdout: fs::read(stdout)?,
        stderr: fs::read(stderr)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn files_hold_the_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = sh("echo out; echo err >&2");
        let ran = run_to_files(&mut command, dir.path(), Duration::from_secs(60))
            .unwrap()
            .unwrap();
        assert_eq!(
            (ran.stdout, ran.stderr),
            (b"out\n".to_vec(), b"err\n".to_vec())
        );
    }
}
