//! Running per-person and per-tile work on several threads without giving
//! up determinism.
//!
//! [`for_each`] hands each item to `f` exactly once; [`run`] hands each
//! part (typically same-sized chunks of several per-tile arrays, see
//! [`chunk_len`]) to `f` exactly once. As long as `f` only touches its own
//! item or part (and reads shared data), the result is the same for any
//! number of threads, because nothing can see another's changes. Anything
//! involving two people (or neighboring tiles) belongs in a sequential step
//! that goes through them in a fixed order.

use std::sync::atomic::{AtomicUsize, Ordering};

use bevy_ecs::prelude::Resource;

/// How many threads per-person and per-tile work may use. 1 runs everything on the
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

/// Below this many tiles, a thread costs more than it saves on cheap
/// per-tile work (starting threads takes about a tenth of a millisecond).
/// Only a speed setting: see [`set_min_tiles_per_thread`].
static MIN_TILES_PER_THREAD: AtomicUsize = AtomicUsize::new(32_768);

/// The fewest tiles worth a thread of their own in per-tile work.
pub fn min_tiles_per_thread() -> usize {
    MIN_TILES_PER_THREAD.load(Ordering::Relaxed)
}

/// Changes [`min_tiles_per_thread`] for the whole process. Results do not
/// depend on it; tests lower it to split even small maps across threads.
#[doc(hidden)]
pub fn set_min_tiles_per_thread(tiles: usize) {
    MIN_TILES_PER_THREAD.store(tiles.max(1), Ordering::Relaxed);
}

/// Length of the chunks that split `len` items across up to `threads`
/// threads, with at least `min_per_thread` items each (all of them in one
/// chunk when there are too few to be worth it).
pub fn chunk_len(len: usize, threads: Threads, min_per_thread: usize) -> usize {
    let threads = threads.0.min(len / min_per_thread.max(1)).max(1);
    len.div_ceil(threads).max(1)
}

/// Calls `f` on every part, each on its own thread (the first on the
/// calling thread).
pub fn run<P: Send>(parts: impl IntoIterator<Item = P>, f: impl Fn(P) + Sync) {
    let mut parts = parts.into_iter().peekable();
    let Some(first) = parts.next() else {
        return;
    };
    if parts.peek().is_none() {
        return f(first);
    }
    let f = &f;
    std::thread::scope(|scope| {
        for part in parts {
            scope.spawn(move || f(part));
        }
        f(first);
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
