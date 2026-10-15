//! Waiting for a child within a time limit, and reading its pipes as it
//! runs, so neither side of a pipe blocks the other.

use std::io::{self, Read};
use std::process::{Child, ExitStatus};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How `child` ended, looking every `poll`: `Ok` on its own, or `Err` as
/// it ended once killed for running past `limit`. A failure to look kills
/// it too, so its readers see the pipes close.
pub fn wait_within(
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
pub fn read_all(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}
