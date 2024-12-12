use crate::{NumBytes, Number};
use backtrace::Backtrace;
use parking_lot::Mutex;
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::LazyLock;

// Portions of this file are derived from the re_memory crate
// which is Copyright (c) 2022 Rerun Technologies AB <opensource@rerun.io>
// and used under the terms of its MIT License
// <https://github.com/rerun-io/rerun/tree/main/crates/utils/re_memory>

thread_local! {
    static IN_TRACKER: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
pub struct TrackingAllocator<A: GlobalAlloc> {
    allocator: A,
}

impl<A: GlobalAlloc> TrackingAllocator<A> {
    pub const fn new(allocator: A) -> Self {
        Self { allocator }
    }
}

static STATS: Stats = Stats::new();

const SMALL_SIZE: usize = 128;
const MEDIUM_SIZE: usize = 4 * 1024;

const MEDIUM_RATE: u64 = 64;
const BIG_RATE: u64 = 1;

static BIG_TRACKER: LazyLock<Mutex<AllocationTracker>> =
    LazyLock::new(|| Mutex::new(AllocationTracker::default()));
static MEDIUM_TRACKER: LazyLock<Mutex<AllocationTracker>> =
    LazyLock::new(|| Mutex::new(AllocationTracker::default()));

// SAFETY: we're passing through the unsafe portions to an underlying
// allocator, which we're relying on to uphold safety.
// The additional logic we add here is safe and is merely tracking
unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.allocator.alloc(layout) };
        track_allocation(ptr, layout.size());
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.allocator.alloc_zeroed(layout) };
        track_allocation(ptr, layout.size());
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.allocator.dealloc(ptr, layout) };
        track_dealloc(ptr, layout.size());
    }

    unsafe fn realloc(&self, old_ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        track_dealloc(old_ptr, layout.size());

        let new_ptr = unsafe { self.allocator.realloc(old_ptr, layout, new_size) };

        track_allocation(new_ptr, new_size);
        new_ptr
    }
}

fn track_allocation(ptr: *mut u8, size: usize) {
    STATS.live.add(size);

    if !STATS.track_callstacks.load(Relaxed) {
        return;
    }

    if size < SMALL_SIZE {
        STATS.small.add(size);
        return;
    }

    IN_TRACKER.with(|in_track| {
        if !in_track.get() {
            in_track.set(true);

            let hash = PtrHash::new(ptr);
            let track = hash.should_sample_size(size);

            if track {
                let bt = Backtrace::new_unresolved();
                if size < MEDIUM_SIZE {
                    STATS.medium.add(size);
                    MEDIUM_TRACKER.lock().track_allocation(hash, size, bt);
                } else {
                    STATS.large.add(size);
                    BIG_TRACKER.lock().track_allocation(hash, size, bt);
                }
            }

            in_track.set(false);
        }
    });
}

fn track_dealloc(ptr: *mut u8, size: usize) {
    STATS.live.sub(size);

    if !STATS.track_callstacks.load(Relaxed) {
        return;
    }

    if size < SMALL_SIZE {
        STATS.small.sub(size);
        return;
    }

    IN_TRACKER.with(|in_track| {
        if !in_track.get() {
            in_track.set(true);

            let hash = PtrHash::new(ptr);
            let track = hash.should_sample_size(size);

            if track {
                if size < MEDIUM_SIZE {
                    MEDIUM_TRACKER.lock().track_dealloc(hash, size);
                    STATS.medium.sub(size);
                } else {
                    STATS.large.sub(size);
                    BIG_TRACKER.lock().track_dealloc(hash, size);
                }
            }

            in_track.set(false);
        }
    });
}

/// Returns the stochastic sampling rate (really, an interval)
/// that should be used for a given allocation size.
fn stochastic_rate_by_size(size: usize) -> u64 {
    if size < MEDIUM_SIZE {
        MEDIUM_RATE
    } else {
        BIG_RATE
    }
}

/// Given a pointer address, hash it into a 64-bit hash value.
/// The hash re-distributes the bits which is important for
/// the stochastic sampling approach used in this module.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
struct PtrHash(u64);

impl PtrHash {
    #[inline]
    pub fn new(ptr: *mut u8) -> Self {
        Self(ahash::RandomState::with_seeds(1, 2, 3, 4).hash_one(ptr))
    }

    /// Given an allocation size, returns true if we should sample
    /// the associated allocation call stack based on the stochastic
    /// rate configured for that allocation size.
    pub fn should_sample_size(&self, size: usize) -> bool {
        let rate = stochastic_rate_by_size(size);
        self.should_sample_at_rate(rate)
    }

    /// Apply "stochastic sampling" at a specified "rate".
    /// The rate is nominally a sampling interval.
    /// The redistribution of the address bits by the hash
    /// "randomizes" the bits and the rate/interval is used
    /// as a mask
    pub fn should_sample_at_rate(&self, rate: u64) -> bool {
        self.0 & (rate - 1) == 0
    }
}

struct CallstackEntry {
    size: usize,
    bt: Backtrace,
}

pub struct CallstackStats {
    pub count: usize,
    pub total_size: usize,
    pub bt: Backtrace,
    pub stochastic_rate: usize,
}

#[derive(Default)]
struct AllocationTracker {
    live_allocations: ahash::HashMap<PtrHash, CallstackEntry>,
}

impl AllocationTracker {
    pub fn track_allocation(&mut self, ptr: PtrHash, size: usize, bt: Backtrace) {
        self.live_allocations
            .insert(ptr, CallstackEntry { size, bt });
    }

    pub fn track_dealloc(&mut self, ptr: PtrHash, _size: usize) {
        self.live_allocations.remove(&ptr);
    }

    pub fn top_callstacks(&self, max_stacks: usize) -> Vec<CallstackStats> {
        let mut by_stack = HashMap::new();

        for alloc in self.live_allocations.values() {
            let key = alloc.bt.frames().iter().map(|f| f.ip()).collect::<Vec<_>>();
            let entry = by_stack.entry(key).or_insert_with(|| CallstackStats {
                count: 0,
                total_size: 0,
                bt: alloc.bt.clone(),
                stochastic_rate: stochastic_rate_by_size(alloc.size) as usize,
            });

            entry.count += 1;
            entry.total_size += alloc.size;
        }

        let mut stats = by_stack
            .into_iter()
            .map(|(_, stats)| stats)
            .collect::<Vec<_>>();
        stats.sort_by(|a, b| b.total_size.cmp(&a.total_size));
        stats.truncate(max_stacks);
        stats.shrink_to_fit();
        stats
    }
}

struct AtomicCountAndSize {
    /// Number of allocations.
    pub count: AtomicUsize,

    /// Number of bytes.
    pub size: AtomicUsize,
}

impl AtomicCountAndSize {
    pub const fn zero() -> Self {
        Self {
            count: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
        }
    }

    fn load(&self) -> CountAndSize {
        CountAndSize {
            count: self.count.load(Relaxed).into(),
            size: self.size.load(Relaxed).into(),
        }
    }

    /// Add an allocation.
    fn add(&self, size: usize) {
        self.count.fetch_add(1, Relaxed);
        self.size.fetch_add(size, Relaxed);
    }

    /// Remove an allocation.
    fn sub(&self, size: usize) {
        self.count.fetch_sub(1, Relaxed);
        self.size.fetch_sub(size, Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CountAndSize {
    pub count: Number,
    pub size: NumBytes,
}

struct Stats {
    live: AtomicCountAndSize,
    track_callstacks: AtomicBool,
    small: AtomicCountAndSize,
    medium: AtomicCountAndSize,
    large: AtomicCountAndSize,
}

impl Stats {
    const fn new() -> Self {
        Self {
            live: AtomicCountAndSize::zero(),
            small: AtomicCountAndSize::zero(),
            medium: AtomicCountAndSize::zero(),
            large: AtomicCountAndSize::zero(),
            track_callstacks: AtomicBool::new(false),
        }
    }
}

/// Number of bytes allocated via the global allocator.
/// Not all of these may be resident; the RSS value will
/// typically be different from this value.
pub fn counted_usage() -> usize {
    STATS.live.size.load(Relaxed)
}

pub fn set_tracking_callstacks(enable: bool) {
    STATS.track_callstacks.store(enable, Relaxed);
}

pub struct TrackingStats {
    pub small_threshold: NumBytes,
    pub live: CountAndSize,
    pub top_callstacks: Vec<CallstackStats>,
}

pub fn tracking_stats() -> TrackingStats {
    const MAX_STACKS: usize = 128;

    let mut top_callstacks = vec![];

    IN_TRACKER.with(|in_track| {
        if !in_track.get() {
            in_track.set(true);
            top_callstacks = BIG_TRACKER.lock().top_callstacks(MAX_STACKS);
            top_callstacks.append(&mut MEDIUM_TRACKER.lock().top_callstacks(MAX_STACKS));

            // Resolve symbols while we are in_track so that the allocations
            // made by this don't "pollute" the overall set of callstacks
            for stack in &mut top_callstacks {
                stack.bt.resolve();
            }

            in_track.set(false);
        }
    });

    TrackingStats {
        small_threshold: SMALL_SIZE.into(),
        live: STATS.live.load(),
        top_callstacks,
    }
}
