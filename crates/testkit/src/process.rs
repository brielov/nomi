//! Running a child within a time limit, with its input fed and its pipes
//! read as it runs, so neither side of a pipe blocks the other; reading a
//! running child's lines as they come; and running a program in a process
//! group that the test, however it ends, stops whole.

use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, Read, Write as _};
use std::net::TcpListener;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use rustix::process::Pid;
#[cfg(unix)]
pub use rustix::process::Signal;

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

/// Each line `read` gives, as it comes, read on a thread of its own.
pub fn lines(read: impl Read + Send + 'static) -> Receiver<String> {
    let (send, lines) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(read).lines() {
            let Ok(line) = line else { break };
            if send.send(line).is_err() {
                break;
            }
        }
    });
    lines
}

/// Waits for a line of `lines` holding `wanted`, failing after a minute;
/// the lines before it. A [`Group`] the lines come from is stopped as the
/// test fails.
pub fn expect(lines: &Receiver<String>, wanted: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut before = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) if line.contains(wanted) => return before,
            Ok(line) => before.push(line),
            Err(e) => panic!("no line with {wanted:?} ({e}); before it: {before:?}"),
        }
    }
}

/// The next `count` lines of `lines`, which may come in any order,
/// sorted; failing after a minute.
pub fn next_lines(lines: &Receiver<String>, count: usize) -> Vec<String> {
    let mut next: Vec<String> = (0..count)
        .map(|_| {
            (lines.recv_timeout(Duration::from_secs(60))).unwrap_or_else(|e| panic!("no line: {e}"))
        })
        .collect();
    next.sort();
    next
}

/// A port on 127.0.0.1 nothing listens on just now, and none this
/// process has given before: another process may still take it before
/// the test binds it, but no two tests here share one.
pub fn free_port() -> u16 {
    static GIVEN: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());
    loop {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port is free");
        let port = listener.local_addr().expect("it is bound").port();
        if GIVEN
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(port)
        {
            return port;
        }
    }
}

/// A program a test started in a process group of its own, which is
/// stopped with every process of the group however the test ends: sent
/// SIGTERM, then SIGKILL once [`Group::GRACE`] has passed, so nothing it
/// started outlives the test.
#[cfg(unix)]
#[derive(Debug)]
pub struct Group {
    child: Child,
}

#[cfg(unix)]
impl Group {
    /// How long the group has to stop on SIGTERM before it is killed.
    const GRACE: Duration = Duration::from_secs(20);

    /// Starts `command` as the leader of a new process group.
    pub fn spawn(command: &mut Command) -> Group {
        use std::os::unix::process::CommandExt as _;
        let child = command
            .process_group(0)
            .spawn()
            .expect("the program starts");
        Group { child }
    }

    /// The leader's process id.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The leader, as started.
    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Each line of the leader's standard output, which is piped, as it
    /// comes.
    pub fn stdout(&mut self) -> Receiver<String> {
        lines(self.child.stdout.take().expect("standard output is piped"))
    }

    /// Each line of the leader's standard error, which is piped, as it
    /// comes.
    pub fn stderr(&mut self) -> Receiver<String> {
        lines(self.child.stderr.take().expect("standard error is piped"))
    }

    /// Sends the leader alone `signal`.
    pub fn signal(&self, signal: Signal) {
        rustix::process::kill_process(self.pid(), signal).expect("the leader is signalled");
    }

    /// The leader's exit code once it ends, failing if it hasn't within
    /// 30 seconds; `None` where a signal ended it.
    pub fn ended(&mut self) -> Option<i32> {
        let status = wait_for(&mut self.child, Duration::from_secs(30))
            .unwrap_or_else(|| panic!("the program is still running after 30 seconds"));
        status.code()
    }

    fn pid(&self) -> Pid {
        Pid::from_child(&self.child)
    }
}

#[cfg(unix)]
impl Drop for Group {
    fn drop(&mut self) {
        use rustix::process::{kill_process_group, test_kill_process_group};
        let group = self.pid();
        // The leader stays unreaped until the group is signalled, so its
        // id can't name another group meanwhile.
        let _ = kill_process_group(group, Signal::TERM);
        let deadline = Instant::now() + Self::GRACE;
        let _ = wait_for(&mut self.child, Self::GRACE);
        while test_kill_process_group(group).is_ok() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = kill_process_group(group, Signal::KILL);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How `child` ended, if it has within `limit`.
#[cfg(unix)]
fn wait_for(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => return None,
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
