use crossbeam_utils::CachePadded;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;

const SHARDS: usize = 256;

struct AtomicCountAndSize {
    count: AtomicUsize,
    size: AtomicUsize,
}

impl AtomicCountAndSize {
    const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
        }
    }
}

/// Sampled totals for live items.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountAndSize {
    pub count: usize,
    pub size: usize,
}

/// Reduces contention when counting live items and their total size across
/// threads.
///
/// Updates are keyed by item identity, allowing different threads to add and
/// remove an item without accumulating per-thread imbalances. Construction and
/// updates never allocate memory.
pub struct ShardedCountAndSize {
    shards: [CachePadded<AtomicCountAndSize>; SHARDS],
}

impl Default for ShardedCountAndSize {
    fn default() -> Self {
        Self::new()
    }
}

impl ShardedCountAndSize {
    /// Creates an empty counter, suitable for static initialization.
    pub const fn new() -> Self {
        Self {
            shards: [const { CachePadded::new(AtomicCountAndSize::new()) }; SHARDS],
        }
    }

    #[inline]
    fn shard_index(key: usize) -> usize {
        // Allocation alignment can make the low address bits identical.
        let mut bits = key as u64;
        bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((bits ^ (bits >> 31)) as usize) & (SHARDS - 1)
    }

    /// Adds an item. Use the same key and size when removing it.
    ///
    /// The live item count and combined size must not exceed `usize::MAX`.
    /// Updates wrap on overflow.
    #[inline]
    pub fn add(&self, key: usize, size: usize) {
        let shard = &self.shards[Self::shard_index(key)];
        shard.count.fetch_add(1, Relaxed);
        shard.size.fetch_add(size, Relaxed);
    }

    /// Removes a previously added item, using its original key and size.
    #[inline]
    pub fn sub(&self, key: usize, size: usize) {
        let shard = &self.shards[Self::shard_index(key)];
        shard.count.fetch_sub(1, Relaxed);
        shard.size.fetch_sub(size, Relaxed);
    }

    /// Returns sampled totals, exact when updates are idle.
    ///
    /// Concurrent updates may make the count and size each overestimate or
    /// underestimate the true value independently. Totals saturate at
    /// `usize::MAX` instead of overflowing.
    pub fn load(&self) -> CountAndSize {
        self.shards
            .iter()
            .fold(CountAndSize::default(), |total, shard| CountAndSize {
                count: total.count.saturating_add(shard.count.load(Relaxed)),
                size: total.size.saturating_add(shard.size.load(Relaxed)),
            })
    }

    /// Returns the sampled total size, with the same semantics as [`Self::load`].
    pub fn load_size(&self) -> usize {
        self.shards.iter().fold(0_usize, |total, shard| {
            total.saturating_add(shard.size.load(Relaxed))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    #[test]
    fn count_and_size() {
        let counter = ShardedCountAndSize::new();
        counter.add(42, 100);
        counter.add(17, 99);
        k9::assert_equal!(
            counter.load(),
            CountAndSize {
                count: 2,
                size: 199
            }
        );
        counter.sub(42, 100);
        k9::assert_equal!(counter.load(), CountAndSize { count: 1, size: 99 });
        counter.sub(17, 99);
        k9::assert_equal!(
            (counter.load(), counter.load_size()),
            (CountAndSize::default(), 0)
        );
    }

    #[test]
    fn cross_thread_removal_and_reuse() {
        let counter = ShardedCountAndSize::new();
        for _ in 0..32 {
            for key in 1..=64 {
                counter.add(key, key);
            }
            thread::scope(|scope| {
                for key in 1..=64 {
                    let counter = &counter;
                    scope.spawn(move || counter.sub(key, key));
                }
            });
            k9::assert_equal!(
                (counter.load(), counter.load_size()),
                (CountAndSize::default(), 0)
            );
        }
    }

    #[test]
    fn aligned_keys_use_all_shards() {
        for alignment in [16, 4096, 65536] {
            let mut seen = [false; SHARDS];
            for key in 1..=8192 {
                seen[ShardedCountAndSize::shard_index(key * alignment)] = true;
            }
            k9::assert_equal!(seen, [true; SHARDS]);
        }
    }

    #[test]
    fn aggregation_saturates() {
        let counter = ShardedCountAndSize::new();
        counter.shards[0].count.store(usize::MAX, Relaxed);
        counter.shards[0].size.store(usize::MAX, Relaxed);
        counter.shards[1].count.store(1, Relaxed);
        counter.shards[1].size.store(1, Relaxed);
        k9::assert_equal!(
            (counter.load(), counter.load_size()),
            (
                CountAndSize {
                    count: usize::MAX,
                    size: usize::MAX
                },
                usize::MAX
            )
        );
    }

    #[test]
    fn concurrent_reads_remain_bounded() {
        let counter = ShardedCountAndSize::new();
        let done = AtomicBool::new(false);
        thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Relaxed) {
                    let value = counter.load();
                    assert!(value.count <= 8);
                    assert!(value.size <= 8 * 256);
                }
            });
            let mut workers = Vec::new();
            for worker in 1..=8 {
                let counter = &counter;
                workers.push(scope.spawn(move || {
                    for _ in 0..100_000 {
                        counter.add(worker * 4096, 256);
                        counter.sub(worker * 4096, 256);
                    }
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
            done.store(true, Relaxed);
        });
        k9::assert_equal!(
            (counter.load(), counter.load_size()),
            (CountAndSize::default(), 0)
        );
    }
}
