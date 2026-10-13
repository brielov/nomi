//! Scaling benchmarks: each times work on inputs of several sizes, each
//! twice the one before, and fails when twice the input takes
//! [`SCALING`] times as long or more. Times are the fastest of several
//! runs.

use std::time::{Duration, Instant};

/// How much slower twice the input may be.
pub const SCALING: f64 = 3.0;

/// The fastest of runs of `run` taking at least a second together, and at
/// least five, after one to warm up.
pub fn fastest(mut run: impl FnMut()) -> Duration {
    run();
    let (mut best, mut total, mut runs) = (Duration::MAX, Duration::ZERO, 0);
    while runs < 5 || total < Duration::from_secs(1) {
        let start = Instant::now();
        run();
        let took = start.elapsed();
        best = best.min(took);
        total += took;
        runs += 1;
    }
    best
}

/// The most any time in `times` grew over the one before it.
pub fn ratio(times: &[Duration]) -> f64 {
    (times.windows(2))
        .map(|pair| pair[1].as_secs_f64() / pair[0].as_secs_f64())
        .fold(0.0, f64::max)
}

/// Ends the benchmark with a failure if any of `slow`, the shapes whose
/// [`ratio`] reached [`SCALING`], is there.
pub fn fail_if_slow(slow: &[&str]) {
    if !slow.is_empty() {
        eprintln!(
            "twice the input took {SCALING} times as long or more: {}",
            slow.join(", ")
        );
        std::process::exit(1);
    }
}

/// `bytes` in whole KiB, for a table.
pub fn kib(bytes: usize) -> String {
    format!("{} KiB", bytes / 1024)
}

/// `time` in milliseconds, for a table.
pub fn ms(time: Duration) -> String {
    format!("{:.2} ms", time.as_secs_f64() * 1e3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ratio_is_the_largest_step() {
        let s = Duration::from_secs;
        assert_eq!(ratio(&[s(1), s(3)]), 3.0);
        assert_eq!(ratio(&[s(2), s(4), s(10)]), 2.5);
        assert_eq!(kib(3000), "2 KiB");
        assert_eq!(ms(Duration::from_micros(1500)), "1.50 ms");
    }
}
