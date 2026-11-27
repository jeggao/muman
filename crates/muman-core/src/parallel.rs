//! A fixed number of threads working through a list, for the steps
//! that spend their time waiting on subprocesses, and for work over a
//! whole library that keeps every core busy.
//!
//! A long run hands on its results a chunk at a time ([`chunked`]), so
//! what it has done is kept as it goes. The workers do not wait for a
//! chunk to be handed on before starting the next: one source can take
//! a hundred times another, as an hour-long video and a cover do, and
//! waiting for each chunk's slowest left every other worker idle at
//! every chunk's end. They run at most one chunk ahead of what has been
//! handed on, so the results held and the work done past a step that
//! fails stay a chunk.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};

/// Concurrent network lookups; more trips YouTube's rate limit sooner
/// than it saves time.
pub const LOOKUPS: usize = 4;

/// One worker per core: a library build is a few short ffmpeg runs.
#[must_use]
pub fn builds() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

/// `f` over every item, `workers` at a time, results in item order.
pub fn map<T: Sync, U: Send>(items: &[T], workers: usize, f: impl Fn(&T) -> U + Sync) -> Vec<U> {
    let workers = workers.clamp(1, items.len().max(1));
    if workers == 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<U>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = f(item);
                    if let Ok(mut slot) = slots[i].lock() {
                        *slot = Some(result);
                    }
                }
            });
        }
    });
    slots
        .into_iter()
        .filter_map(|slot| slot.into_inner().ok().flatten())
        .collect()
}

/// `f` over `items` in chunks of a few per worker, `workers` at a time,
/// each chunk's results handed to `on_chunk` in item order, the workers
/// running on into the chunk after it meanwhile but no further; an error
/// from `on_chunk` starts no more and is returned once those under way
/// end.
pub fn chunked<T: Sync, U: Send, E>(
    items: &[T],
    workers: usize,
    f: impl Fn(&T) -> U + Sync,
    mut on_chunk: impl FnMut(&[T], Vec<U>) -> Result<(), E>,
) -> Result<(), E> {
    let size = workers.max(1) * 8;
    let workers = workers.clamp(1, items.len().max(1));
    if workers == 1 {
        for chunk in items.chunks(size) {
            on_chunk(chunk, chunk.iter().map(&f).collect())?;
        }
        return Ok(());
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    // The first item no worker may start yet.
    let limit = Mutex::new(2 * size);
    let moved = Condvar::new();
    std::thread::scope(|scope| {
        let (sent, received) = std::sync::mpsc::channel();
        for _ in 0..workers {
            let sent = sent.clone();
            let (next, stop, limit, moved, f) = (&next, &stop, &limit, &moved, &f);
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let mut open = limit.lock().unwrap_or_else(PoisonError::into_inner);
                    while i >= *open && !stop.load(Ordering::Relaxed) {
                        open = moved.wait(open).unwrap_or_else(PoisonError::into_inner);
                    }
                    drop(open);
                    if stop.load(Ordering::Relaxed) || sent.send((i, f(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sent);
        let mut done: BTreeMap<usize, U> = BTreeMap::new();
        let mut handed = Ok(());
        // A worker that panicked never sends its item: the chunk waiting
        // for it ends when every worker has, and the scope panics with it.
        'chunks: for (n, chunk) in items.chunks(size).enumerate() {
            let start = n * size;
            while !(start..start + chunk.len()).all(|i| done.contains_key(&i)) {
                match received.recv() {
                    Ok((i, u)) => {
                        done.insert(i, u);
                    }
                    Err(_) => break 'chunks,
                }
            }
            let results = (start..start + chunk.len())
                .filter_map(|i| done.remove(&i))
                .collect();
            handed = on_chunk(chunk, results);
            if handed.is_err() {
                break;
            }
            *limit.lock().unwrap_or_else(PoisonError::into_inner) = start + chunk.len() + 2 * size;
            moved.notify_all();
        }
        stop.store(true, Ordering::Relaxed);
        moved.notify_all();
        handed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_the_items_order() {
        let items: Vec<u64> = (0..50).collect();
        let out = map(&items, 8, |n| {
            std::thread::sleep(std::time::Duration::from_millis(50 - n));
            n * 2
        });
        assert_eq!(out, items.iter().map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn chunks_are_handed_on_in_order_with_work_at_most_a_chunk_ahead() {
        let items: Vec<u64> = (0..50).collect();
        let started = std::sync::atomic::AtomicUsize::new(0);
        let mut seen = Vec::new();
        let mut sizes = Vec::new();
        chunked(
            &items,
            2,
            |n| {
                started.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(n % 3));
                n * 2
            },
            |chunk, done| {
                let ahead = started.load(Ordering::Relaxed) - seen.len() - chunk.len();
                assert!(ahead <= 16, "{ahead} started past the next chunk");
                sizes.push(chunk.len());
                seen.extend(done);
                Ok::<(), ()>(())
            },
        )
        .unwrap();
        assert_eq!(seen, items.iter().map(|n| n * 2).collect::<Vec<_>>());
        assert_eq!(sizes, [16, 16, 16, 2], "the chunks as before");
    }

    #[test]
    fn a_chunk_refused_starts_no_work_past_the_next() {
        let items: Vec<u64> = (0..200).collect();
        let started = std::sync::atomic::AtomicUsize::new(0);
        let refused = chunked(
            &items,
            2,
            |_| {
                started.fetch_add(1, Ordering::Relaxed);
            },
            |chunk, _| if chunk[0] == 16 { Err("full") } else { Ok(()) },
        );
        assert_eq!(refused, Err("full"));
        assert!(started.load(Ordering::Relaxed) <= 48, "{started:?}");
    }

    #[test]
    fn work_runs_concurrently() {
        let items = [(); 8];
        let start = std::time::Instant::now();
        map(&items, 8, |()| {
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
        // Eight in a row take 800 ms; a busy CI runner gets some slack.
        assert!(start.elapsed() < std::time::Duration::from_millis(700));
    }
}
