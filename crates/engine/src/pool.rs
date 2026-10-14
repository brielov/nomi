//! The process's one thread pool. Work that runs in parallel runs here, so
//! the thread count is set once and every thread has the stack core work
//! needs.

use std::sync::OnceLock;

use rayon::prelude::*;

/// A pool of worker threads, each with [`nomi_base::STACK_SIZE`] of stack.
/// [`Pool::global`] is the only one; nothing else in nomi starts threads.
#[derive(Debug)]
pub struct Pool(rayon::ThreadPool);

static GLOBAL: OnceLock<Pool> = OnceLock::new();

impl Pool {
    /// The pool, started on first use with a thread per core, or as many
    /// as `RAYON_NUM_THREADS` says.
    pub fn global() -> &'static Pool {
        GLOBAL.get_or_init(|| {
            let pool = rayon::ThreadPoolBuilder::new()
                .stack_size(nomi_base::STACK_SIZE)
                .thread_name(|i| format!("nomi-{i}"))
                .build()
                .expect("the thread pool starts");
            Pool(pool)
        })
    }

    /// How many threads the pool runs.
    pub fn threads(&self) -> usize {
        self.0.current_num_threads()
    }

    /// `f` of every item, run across the pool, in the items' order.
    pub fn map<T, R>(&self, items: Vec<T>, f: impl Fn(T) -> R + Send + Sync) -> Vec<R>
    where
        T: Send,
        R: Send,
    {
        self.0.install(|| items.into_par_iter().map(f).collect())
    }

    /// Frees `value` on the pool, returning at once: a build's big values
    /// take a while to free, which it need not wait for.
    pub(crate) fn drop_later(&self, value: impl Send + 'static) {
        self.0.spawn(move || drop(value));
    }

    /// Runs `f` on the pool, returning once it and all the work it
    /// spawned have finished.
    pub(crate) fn scope<'scope, R>(&self, f: impl FnOnce(&rayon::Scope<'scope>) -> R + Send) -> R
    where
        R: Send,
    {
        self.0.scope(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_the_items_order() {
        let items: Vec<u32> = (0..1000).collect();
        let squares = Pool::global().map(items, |n| n * n);
        assert_eq!(squares, (0..1000).map(|n| n * n).collect::<Vec<_>>());
    }

    #[test]
    fn there_is_one_pool() {
        assert!(std::ptr::eq(Pool::global(), Pool::global()));
        assert!(Pool::global().threads() >= 1);
    }

    /// Recursion that takes about `depth` KiB of stack.
    fn deep(depth: usize) -> u8 {
        let frame = std::hint::black_box([depth as u8; 1024]);
        if depth == 0 {
            frame[0]
        } else {
            deep(depth - 1).wrapping_add(frame[1023])
        }
    }

    #[test]
    fn workers_have_the_core_stack() {
        // At least 12 MiB, more than a main thread's 8 and far past a
        // spawned one's 2; a debug build's frames take up to four times
        // their array.
        let depths = vec![12 * 1024; 2];
        Pool::global().map(depths, deep);
    }
}
