//! A fixed number of threads working through a list, for the steps
//! that spend their time waiting on subprocesses.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

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
