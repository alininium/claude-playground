//! Running per-person work on several threads without giving up
//! determinism.
//!
//! [`for_each`] hands each item to `f` exactly once. As long as `f` only
//! touches its own item (and reads shared data), the result is the same for
//! any number of threads, because no item can see another's changes.
//! Anything involving two people belongs in a sequential resolve step that
//! goes through them in id order.

use bevy_ecs::prelude::Resource;

/// How many threads per-person work may use. 1 runs everything on the
/// calling thread. Never part of the simulated state: any value gives the
/// same results.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Threads(pub usize);

impl Default for Threads {
    fn default() -> Self {
        let n = std::env::var("SOCIETIES_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
        Self(n.max(1))
    }
}

/// Below this many items, threads cost more than they save.
const MIN_PER_THREAD: usize = 512;

/// Calls `f` on every item, splitting the slice across up to `threads`
/// threads.
pub fn for_each<T: Send>(items: &mut [T], threads: Threads, f: impl Fn(&mut T) + Sync) {
    let threads = threads.0.min(items.len() / MIN_PER_THREAD).max(1);
    if threads == 1 {
        items.iter_mut().for_each(f);
        return;
    }
    let chunk = items.len().div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        for part in items.chunks_mut(chunk) {
            scope.spawn(move || part.iter_mut().for_each(f));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_result_on_any_number_of_threads() {
        let base: Vec<u64> = (0..10_000).collect();
        let run = |threads| {
            let mut v = base.clone();
            for_each(&mut v, Threads(threads), |x| {
                *x = crate::people::rng::mix(*x, 1)
            });
            v
        };
        let one = run(1);
        for t in [2, 3, 8] {
            assert_eq!(run(t), one);
        }
    }
}
