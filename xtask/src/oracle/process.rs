//! Running an upstream tool within a time limit, with its input fed and
//! its output read as it runs, so neither side of a pipe blocks the other.

use std::fs;
use std::io::{self, Read, Write as _};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// How often [`run_within`] looks whether a child with a limit has
/// ended: elm parses a module in milliseconds, and the syntax oracle runs
/// it hundreds of thousands of times; the scoreboard times runs of a few.
const POLL: Duration = Duration::from_millis(1);

/// Runs `command` with `input` as its standard input, fed on a thread of
/// its own, and reads whatever output `command` pipes as it comes, so a
/// child that doesn't read its input, or writes more than a pipe holds,
/// blocks neither. Past `limit` the child is killed and the run is
/// `None`. Output the command doesn't pipe comes back empty. The run
/// lasts until the child's pipes close, which a process it started and
/// left running can hold open.
pub(crate) fn run_within(
    command: &mut Command,
    input: &[u8],
    limit: Option<Duration>,
) -> io::Result<Option<Output>> {
    let stdin = if input.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    };
    let mut child = command.stdin(stdin).spawn()?;
    thread::scope(|scope| {
        if let Some(mut stdin) = child.stdin.take() {
            // A child may exit without reading its input; the write then
            // fails, and dropping the pipe closes it either way.
            scope.spawn(move || stdin.write_all(input));
        }
        let stdout = read_all(scope, child.stdout.take());
        let stderr = read_all(scope, child.stderr.take());
        let status = wait(&mut child, limit);
        let joined = |reader: Option<ScopedJoinHandle<'_, Vec<u8>>>| {
            reader.map_or_else(Vec::new, |r| r.join().expect("a reader panicked"))
        };
        let (stdout, stderr) = (joined(stdout), joined(stderr));
        Ok(status?.map(|status| Output {
            status,
            stdout,
            stderr,
        }))
    })
}

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
    let Some(ended) = run_within(command, &[], Some(limit)).with_context(|| program)? else {
        return Ok(None);
    };
    Ok(Some(Output {
        status: ended.status,
        stdout: fs::read(stdout)?,
        stderr: fs::read(stderr)?,
    }))
}

/// Everything `pipe` gives, if there is one, read on a thread of `scope`.
fn read_all<'scope>(
    scope: &'scope Scope<'scope, '_>,
    pipe: Option<impl Read + Send + 'scope>,
) -> Option<ScopedJoinHandle<'scope, Vec<u8>>> {
    pipe.map(|mut pipe| {
        scope.spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    })
}

/// How `child` ended, or `None` once it has run past `limit` and been
/// killed.
fn wait(child: &mut Child, limit: Option<Duration>) -> io::Result<Option<ExitStatus>> {
    let Some(limit) = limit else {
        return child.wait().map(Some);
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) if started.elapsed() <= limit => thread::sleep(POLL),
            // Killed either way, so the readers see the pipes close.
            ended => {
                child.kill()?;
                child.wait()?;
                return ended.map(|_| None);
            }
        }
    }
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
    fn a_child_that_never_reads_ends_at_the_limit() {
        let started = Instant::now();
        let input = vec![b'x'; 1 << 20];
        let mut command = Command::new("sleep");
        command.arg("30");
        let ran = run_within(&mut command, &input, Some(Duration::from_millis(200))).unwrap();
        assert!(ran.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_child_that_floods_its_output_is_read_whole() {
        let input = vec![b'x'; 1 << 20];
        let mut command = sh("cat; head -c 1048576 /dev/zero >&2");
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let ran = run_within(&mut command, &input, Some(Duration::from_secs(60)))
            .unwrap()
            .expect("it ends");
        assert!(ran.status.success());
        assert_eq!(ran.stdout, input);
        assert_eq!(ran.stderr.len(), 1 << 20);
    }

    #[test]
    fn output_not_piped_comes_back_empty() {
        let mut command = sh("echo out; echo err >&2");
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        let ran = run_within(&mut command, b"", None).unwrap().unwrap();
        assert_eq!((ran.stdout, ran.stderr), (Vec::new(), b"err\n".to_vec()));
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
