//! Running a child within a time limit, with its input fed and its pipes
//! read as it runs, so neither side of a pipe blocks the other.

use std::io::{self, Read, Write as _};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often [`run_within`] looks whether a child with a limit has
/// ended: elm parses a module in milliseconds, and the syntax oracle runs
/// it hundreds of thousands of times; the scoreboard times runs of a few.
const POLL: Duration = Duration::from_millis(1);

/// Runs `command` with `input` as its standard input, fed on a thread of
/// its own, and reads whatever output `command` pipes as it comes, so a
/// child that doesn't read its input, or writes more than a pipe holds,
/// blocks neither: `Ok` as it ended on its own, or `Err` as it ended once
/// killed for running past `limit`. Output the command doesn't pipe comes
/// back empty. The run lasts until the child's pipes close, which a
/// process it started and left running can hold open.
pub fn run_within(
    command: &mut Command,
    input: &[u8],
    limit: Option<Duration>,
) -> io::Result<Result<Output, Output>> {
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
        let stdout = child.stdout.take().map(read_all);
        let stderr = child.stderr.take().map(read_all);
        let status = match limit {
            None => child.wait().map(Ok),
            Some(limit) => wait_within(&mut child, limit, POLL),
        };
        let joined = |reader: Option<JoinHandle<Vec<u8>>>| {
            reader.map_or_else(Vec::new, |r| r.join().expect("a reader panicked"))
        };
        let (stdout, stderr) = (joined(stdout), joined(stderr));
        Ok(match status? {
            Ok(status) => Ok(Output {
                status,
                stdout,
                stderr,
            }),
            Err(status) => Err(Output {
                status,
                stdout,
                stderr,
            }),
        })
    })
}

/// How `child` ended, looking every `poll`: `Ok` on its own, or `Err` as
/// it ended once killed for running past `limit`. A failure to look kills
/// it too, so its readers see the pipes close.
pub(crate) fn wait_within(
    child: &mut Child,
    limit: Duration,
    poll: Duration,
) -> io::Result<Result<ExitStatus, ExitStatus>> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Ok(status)),
            Ok(None) if started.elapsed() <= limit => thread::sleep(poll),
            looked => {
                child.kill()?;
                let killed = child.wait()?;
                return looked.map(|_| Err(killed));
            }
        }
    }
}

/// Everything `pipe` gives, read on a thread of its own.
fn read_all(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
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
        assert!(ran.is_err());
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
}
