//! Room on the stack for deep recursion. Recursive passes guard each
//! step with [`grow`] or [`grow_large`], so the depth the parser admits
//! is bounded by memory, not by the thread's stack, as GHC's growing
//! stack bounds upstream's.

/// The stack every thread that runs core work needs, and gets from the
/// shell. Recursive passes guard themselves with `stacker`, but rowan
/// drops a syntax tree recursively, so this must hold the drop of the
/// deepest tree the parser admits; it is reserved, not committed, so the
/// size costs nothing until used.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;

/// Runs `step`, first moving to a new 1 MiB segment of stack if less
/// than 64 KiB is left: enough for a step whose frames, up to the next
/// guarded step, are small.
pub fn grow<R>(step: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, step)
}

/// [`grow`] for a step with large frames, such as a parser rule or a
/// lowering pass's, which keeps 128 KiB free and grows by 2 MiB.
pub fn grow_large<R>(step: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, step)
}

/// Runs `f` on a stack of [`STACK_SIZE`], the one the shell gives
/// threads that run core work, wherever it is called from.
pub fn on_core_stack<R>(f: impl FnOnce() -> R) -> R {
    stacker::grow(STACK_SIZE, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(n: u32) -> u32 {
        if n == 0 { 0 } else { grow(|| depth(n - 1)) + 1 }
    }

    #[test]
    fn guarded_recursion_outgrows_the_stack_it_started_on() {
        // Far deeper than a test thread's 2 MiB stack holds unguarded.
        assert_eq!(on_core_stack(|| grow_large(|| depth(200_000))), 200_000);
    }
}
