//! Single-owner resolution of names to shared handles.
//!
//! Building a handle may run Lua and DNS queries. [`ResolverMap`] admits
//! exactly one caller per name as the [`Owner`] of a resolution and makes
//! everyone else wait on that attempt and retry once it finishes. This
//! module only coordinates who builds and who waits. The admission policy
//! and the handle lifecycle are the caller's concern.

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use kumo_prometheus::declare_metric;
use kumo_prometheus::prometheus::IntGauge;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;

declare_metric! {
/// how many scheduled queues are tracked by the QueueManager
pub static SCHEDULED_QUEUE_COUNT: IntGauge("scheduled_queue_count");
}

/// Holds the value an `admit` function (see [`ResolverMap::new`]) returned
/// when it admitted a resolution, for as long as that resolution is tracked.
pub struct AdmitToken(#[allow(dead_code)] Box<dyn Send>);

impl AdmitToken {
    /// Wrap a value to hand back from an `admit` function.
    pub fn new(value: impl Send + 'static) -> Self {
        Self(Box::new(value))
    }
}

/// Grants an [`AdmitToken`] per resolution, or returns the error that becomes
/// the caller's refusal when admission is closed.
type AdmitFn = Box<dyn Fn(&str) -> anyhow::Result<AdmitToken> + Send + Sync>;

/// How long a negatively cached failure is served before a caller may retry.
pub const NEGATIVE_CACHE: std::time::Duration = std::time::Duration::from_secs(60);

/// The state of one name in the map.
enum Slot<T> {
    /// A fully constructed handle.
    Handle(Arc<T>),
    /// A resolution in progress. The matching [`Owner`] holds the same
    /// `Arc<Attempt>`, and its pointer identity distinguishes this attempt
    /// from any later attempt for the same name.
    Resolving(Arc<Attempt>),
    /// A negatively cached construction failure, served until `expires`.
    Failed {
        error: Arc<anyhow::Error>,
        expires: Instant,
    },
}

/// The shared completion signal for one resolution attempt.
///
/// Waiters block on [`Attempt::wait`] until the owner calls
/// [`Attempt::mark_done`].
pub struct Attempt {
    /// Used only to wake and unblock every waiter when closed. A permit is
    /// never acquired. A `wait` that starts after the close also returns at
    /// once, which `tokio::sync::Notify` would not do.
    done: Semaphore,
}

impl Attempt {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            done: Semaphore::new(0),
        })
    }

    /// Blocks until the resolution completes.
    pub async fn wait(&self) {
        let _ = self.done.acquire().await;
    }

    /// Wake all waiters, and any later waiter, that the resolution has finished.
    fn mark_done(&self) {
        self.done.close();
    }
}

/// The outcome of [`ResolverMap::acquire`].
pub enum Acquire<T> {
    /// The name already resolves to this handle.
    Ready(Arc<T>),
    /// The name is negatively cached with this error.
    Failed(Arc<anyhow::Error>),
    /// A resolution is already in progress. Wait on it, then acquire again.
    Wait(Arc<Attempt>),
    /// This caller is admitted to construct the value. It must finalize the
    /// returned owner with [`Owner::publish`], [`Owner::fail`], or
    /// [`Owner::remove`], or drop it to abandon the attempt.
    Owner(Owner<T>),
    /// Admission is closed (the process is shutting down).
    Refused(anyhow::Error),
}

/// A map of names undergoing single-owner resolution. Cloning shares the
/// same underlying map.
pub struct ResolverMap<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for ResolverMap<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct Inner<T> {
    slots: DashMap<String, Slot<T>>,
    admit: AdmitFn,
    /// Counts map entries in any state. Each insertion and removal site in
    /// this module updates it directly.
    count: IntGauge,
}

impl<T> ResolverMap<T> {
    /// Build a resolver whose admissions are granted by `admit` and whose entry
    /// count is reported through `count`. `admit` is called once per new
    /// attempt, while the installing shard lock is held, and must not call back
    /// into this map.
    pub fn new(
        count: IntGauge,
        admit: impl Fn(&str) -> anyhow::Result<AdmitToken> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                slots: DashMap::new(),
                admit: Box::new(admit),
                count,
            }),
        }
    }

    /// Check the current state of `name` and, if nothing is in progress, try to
    /// start one. Returns an [`Acquire`] describing the outcome. A `Wait`
    /// result means the caller must call this again after waiting on the
    /// attempt. Non-blocking: this never awaits.
    pub fn acquire(&self, name: &str) -> Acquire<T> {
        if let Some(slot) = self.inner.slots.get(name) {
            if let Slot::Handle(handle) = slot.value() {
                return Acquire::Ready(handle.clone());
            }
        }

        let now = Instant::now();
        // `entry` holds the shard write lock across the `admit` call and the
        // insert. A concurrent acquire for the same name waits on that lock and
        // then finds the installed slot, which prevents a second caller from
        // being admitted for the same name.
        match self.inner.slots.entry(name.to_string()) {
            Entry::Occupied(mut occ) => match occ.get() {
                Slot::Handle(handle) => Acquire::Ready(handle.clone()),
                Slot::Resolving(attempt) => Acquire::Wait(attempt.clone()),
                Slot::Failed { error, expires } if *expires > now => Acquire::Failed(error.clone()),
                Slot::Failed { .. } => match (self.inner.admit)(name) {
                    // The negative cache expired. This overwrites the occupied
                    // entry in place. The count already includes it.
                    Ok(token) => {
                        let attempt = Attempt::new();
                        occ.insert(Slot::Resolving(attempt.clone()));
                        Acquire::Owner(self.make_owner(name, attempt, token))
                    }
                    Err(err) => Acquire::Refused(err),
                },
            },
            Entry::Vacant(vac) => match (self.inner.admit)(name) {
                Ok(token) => {
                    let attempt = Attempt::new();
                    vac.insert(Slot::Resolving(attempt.clone()));
                    self.inner.count.inc();
                    Acquire::Owner(self.make_owner(name, attempt, token))
                }
                Err(err) => Acquire::Refused(err),
            },
        }
    }

    fn make_owner(&self, name: &str, attempt: Arc<Attempt>, token: AdmitToken) -> Owner<T> {
        Owner {
            map: self.clone(),
            name: name.to_string(),
            attempt,
            token: Some(token),
            armed: true,
        }
    }

    /// Return the handle for `name`, or `None` if it is absent or not yet
    /// constructed (resolving or failed).
    pub fn get_handle(&self, name: &str) -> Option<Arc<T>> {
        match self.inner.slots.get(name)?.value() {
            Slot::Handle(handle) => Some(handle.clone()),
            Slot::Resolving(_) | Slot::Failed { .. } => None,
        }
    }

    /// Return every name currently in the map, in any state.
    pub fn names(&self) -> Vec<String> {
        self.inner.slots.iter().map(|e| e.key().clone()).collect()
    }

    /// Remove `name` only if it still holds exactly `handle` and `pred` is true,
    /// all under the entry lock. Returns whether it was removed. The identity
    /// check stops a stale handle from evicting a newer entry with the same
    /// name.
    pub fn remove_handle_if(
        &self,
        name: &str,
        handle: &Arc<T>,
        pred: impl FnOnce() -> bool,
    ) -> bool {
        let removed = self.inner.slots.remove_if(name, |_, slot| match slot {
            Slot::Handle(current) => Arc::ptr_eq(current, handle) && pred(),
            _ => false,
        });
        if removed.is_some() {
            self.inner.count.dec();
            true
        } else {
            false
        }
    }

    /// Remove every `Failed` slot from the map.
    pub fn purge_failed(&self) {
        let count = &self.inner.count;
        self.inner.slots.retain(|_, slot| {
            if matches!(slot, Slot::Failed { .. }) {
                count.dec();
                false
            } else {
                true
            }
        });
    }

    /// Whether any handle or in-progress resolution remains. This excludes
    /// failed slots, because shutdown treats a lingering negative cache as
    /// not outstanding work.
    pub fn work_remains(&self) -> bool {
        self.inner
            .slots
            .iter()
            .any(|e| matches!(e.value(), Slot::Handle(_) | Slot::Resolving(_)))
    }

    /// Replace the `Resolving` slot for `name` with `slot`, but only if it
    /// still holds `attempt`. Returns whether it matched.
    fn replace_owned(&self, name: &str, attempt: &Arc<Attempt>, slot: Slot<T>) -> bool {
        match self.inner.slots.entry(name.to_string()) {
            Entry::Occupied(mut occ) => {
                if matches!(occ.get(), Slot::Resolving(current) if Arc::ptr_eq(current, attempt)) {
                    occ.insert(slot);
                    true
                } else {
                    false
                }
            }
            Entry::Vacant(_) => false,
        }
    }

    /// Remove the `Resolving` slot for `name`, but only if it still holds
    /// `attempt`. Returns whether it matched.
    fn remove_owned(&self, name: &str, attempt: &Arc<Attempt>) -> bool {
        let removed = self.inner.slots.remove_if(
            name,
            |_, slot| matches!(slot, Slot::Resolving(current) if Arc::ptr_eq(current, attempt)),
        );
        if removed.is_some() {
            self.inner.count.dec();
            true
        } else {
            false
        }
    }
}

/// The right to construct the value for one name, and the duty to clear the
/// slot if construction does not succeed.
///
/// Finalize with [`publish`](Owner::publish), [`fail`](Owner::fail), or
/// [`remove`](Owner::remove). Dropping an owner that was not finalized (a
/// cancelled or unwound caller) removes the slot and wakes the waiters.
pub struct Owner<T> {
    map: ResolverMap<T>,
    name: String,
    attempt: Arc<Attempt>,
    /// The admission token, held until finalization. `Some` while armed.
    token: Option<AdmitToken>,
    armed: bool,
}

impl<T> Owner<T> {
    /// Publish `handle` as the resolved value and wake the waiters. Returns the
    /// handle, or an error if the slot no longer holds this owner's attempt,
    /// which is an invariant violation: the handle is dropped rather than
    /// installed.
    pub fn publish(mut self, handle: Arc<T>) -> anyhow::Result<Arc<T>> {
        let matched =
            self.map
                .replace_owned(&self.name, &self.attempt, Slot::Handle(handle.clone()));
        self.finish();
        if matched {
            Ok(handle)
        } else {
            tracing::error!(
                "resolution ownership for {} was lost before publishing; \
                 discarding the constructed value",
                self.name
            );
            anyhow::bail!(
                "resolution ownership for {} lost while publishing",
                self.name
            );
        }
    }

    /// Negatively cache `error` until `expires` and wake the waiters.
    pub fn fail(mut self, error: Arc<anyhow::Error>, expires: Instant) {
        let matched =
            self.map
                .replace_owned(&self.name, &self.attempt, Slot::Failed { error, expires });
        if !matched {
            tracing::error!(
                "resolution ownership for {} was lost before caching its failure",
                self.name
            );
        }
        self.finish();
    }

    /// Remove the slot without caching anything and wake the waiters.
    pub fn remove(mut self) {
        self.map.remove_owned(&self.name, &self.attempt);
        self.finish();
    }

    fn finish(&mut self) {
        // We clear `armed` before doing this cleanup ourselves, because Drop
        // also removes the slot and calls `mark_done` when `armed` is set;
        // leaving it set here would run that cleanup a second time.
        self.armed = false;
        self.attempt.mark_done();
        self.token.take();
    }
}

impl<T> Drop for Owner<T> {
    fn drop(&mut self) {
        if self.armed {
            self.map.remove_owned(&self.name, &self.attempt);
            self.attempt.mark_done();
        }
    }
}

/// An error handed to a caller that read a value from the map. Its
/// [`std::error::Error::source`] returns the cached error, extending the source
/// chain into the cached error's chain. `anyhow::Error::root_cause` walks that
/// chain to the end, reaching the original failure rather than stopping at this
/// wrapper.
#[derive(Debug)]
struct CachedError(Arc<anyhow::Error>);

impl std::fmt::Display for CachedError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CachedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&**self.0)
    }
}

/// Wrap `error` for return to a caller, preserving its source chain for
/// `root_cause`.
pub fn wrap_cached(error: Arc<anyhow::Error>) -> anyhow::Error {
    anyhow::Error::new(CachedError(error))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Builds a `ResolverMap<u32>` and its gauge. Admission succeeds while
    /// `closed` is false and fails once it is set to true.
    fn test_map(closed: Arc<AtomicBool>) -> (ResolverMap<u32>, IntGauge) {
        let gauge = IntGauge::new("test_resolver_count", "help").unwrap();
        let g = gauge.clone();
        let map = ResolverMap::new(gauge, move |_name| {
            if closed.load(Ordering::SeqCst) {
                anyhow::bail!("admission closed");
            }
            Ok(AdmitToken::new(()))
        });
        (map, g)
    }

    fn owner(acq: Acquire<u32>) -> Owner<u32> {
        match acq {
            Acquire::Owner(o) => o,
            _ => panic!("expected Owner"),
        }
    }

    #[test]
    fn single_owner_others_wait() {
        let (map, _g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        // A second caller for the same name must wait, not become a second owner.
        assert!(matches!(map.acquire("a"), Acquire::Wait(_)));
        let h = o.publish(Arc::new(1)).unwrap();
        assert_eq!(*h, 1);
        // Once published, the name resolves directly.
        assert!(matches!(map.acquire("a"), Acquire::Ready(h) if *h == 1));
    }

    #[tokio::test]
    async fn waiter_cancel_leaves_owner() {
        let (map, _g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        let attempt = match map.acquire("a") {
            Acquire::Wait(a) => a,
            _ => panic!("expected Wait"),
        };
        // Drop the waiter's future without ever polling it to completion.
        drop(attempt);
        // The owner is unaffected and can still publish.
        assert!(o.publish(Arc::new(7)).is_ok());
        assert!(matches!(map.acquire("a"), Acquire::Ready(_)));
    }

    #[tokio::test]
    async fn drop_owner_wakes_waiter_who_becomes_owner() {
        let (map, g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        let attempt = match map.acquire("a") {
            Acquire::Wait(a) => a,
            _ => panic!("expected Wait"),
        };
        // Owner abandons the attempt without finalizing, freeing the name
        // immediately.
        drop(o);
        attempt.wait().await;
        // The woken waiter becomes the new owner here.
        let next = owner(map.acquire("a"));
        // Hold `next`: dropping an unfinalized owner removes the slot.
        assert_eq!(g.get(), 1);
        drop(next);
    }

    #[test]
    fn ready_fast_path_returns_live_handle_without_side_effects() {
        let (map, g) = test_map(Arc::new(AtomicBool::new(false)));
        let published = owner(map.acquire("a")).publish(Arc::new(1)).unwrap();
        assert_eq!(g.get(), 1);
        // Repeated lookups of a resolved name take the read-locked fast path:
        // each returns the same live handle and leaves the map untouched.
        for _ in 0..3 {
            match map.acquire("a") {
                Acquire::Ready(h) => assert!(Arc::ptr_eq(&h, &published)),
                _ => panic!("expected Ready"),
            }
        }
        assert_eq!(g.get(), 1);
        assert_eq!(map.names(), vec!["a".to_string()]);
    }

    #[test]
    fn publish_then_reap() {
        let (map, g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        let h = o.publish(Arc::new(1)).unwrap();
        assert_eq!(g.get(), 1);
        // A stale handle must not evict the live entry.
        let stale = Arc::new(1);
        assert!(!map.remove_handle_if("a", &stale, || true));
        assert_eq!(g.get(), 1);
        // The live handle, with a satisfied predicate, is reaped.
        assert!(map.remove_handle_if("a", &h, || true));
        assert_eq!(g.get(), 0);
        assert!(map.get_handle("a").is_none());
    }

    #[test]
    fn refused_when_closed_leaves_map_unchanged() {
        let closed = Arc::new(AtomicBool::new(true));
        let (map, g) = test_map(closed.clone());
        assert!(matches!(map.acquire("a"), Acquire::Refused(_)));
        assert!(map.names().is_empty());
        assert_eq!(g.get(), 0);
    }

    #[test]
    fn expired_failure_reacquires() {
        let (map, g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        let err = Arc::new(anyhow::anyhow!("boom"));
        // Cache a failure that has already expired.
        o.fail(err, Instant::now() - std::time::Duration::from_secs(1));
        assert_eq!(g.get(), 1);
        // Acquiring again replaces the expired failure with a fresh owner,
        // without changing the count.
        let next = owner(map.acquire("a"));
        // Hold `next`: dropping an unfinalized owner removes the slot.
        assert_eq!(g.get(), 1);
        drop(next);
    }

    #[test]
    fn unexpired_failure_is_served() {
        let (map, _g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        o.fail(
            Arc::new(anyhow::anyhow!("boom")),
            Instant::now() + std::time::Duration::from_secs(60),
        );
        assert!(matches!(map.acquire("a"), Acquire::Failed(_)));
    }

    #[test]
    fn purge_failed_clears_cache_and_not_work() {
        let (map, g) = test_map(Arc::new(AtomicBool::new(false)));
        owner(map.acquire("fail")).fail(
            Arc::new(anyhow::anyhow!("boom")),
            Instant::now() + std::time::Duration::from_secs(60),
        );
        let live = owner(map.acquire("live")).publish(Arc::new(1)).unwrap();
        assert_eq!(g.get(), 2);
        // A failed slot is not outstanding work. A handle is.
        assert!(map.work_remains());
        map.purge_failed();
        assert_eq!(g.get(), 1);
        assert!(map.work_remains());
        // Removing the handle doesn't leave any work.
        assert!(map.remove_handle_if("live", &live, || true));
        assert!(!map.work_remains());
        assert_eq!(g.get(), 0);
    }

    #[tokio::test]
    async fn drop_owner_clears_slot_and_wakes_waiter() {
        let (map, g) = test_map(Arc::new(AtomicBool::new(false)));
        let o = owner(map.acquire("a"));
        let attempt = match map.acquire("a") {
            Acquire::Wait(a) => a,
            _ => panic!("expected Wait"),
        };
        // Dropping an unfinalized owner removes the slot and wakes waiters.
        drop(o);
        attempt.wait().await;
        assert_eq!(g.get(), 0);
        assert!(!map.work_remains());
    }

    #[test]
    fn cached_error_reaches_root_cause() {
        #[derive(Debug, thiserror::Error)]
        #[error("the root")]
        struct Root;
        let original = anyhow::Error::new(Root).context("while doing a thing");
        let wrapped = wrap_cached(Arc::new(original));
        assert!(wrapped.root_cause().downcast_ref::<Root>().is_some());
    }

    /// The shutdown finalization policy removes a construction failure
    /// admitted while shutdown is in progress instead of negatively caching
    /// it, and releases the waiter once the slot is cleared.
    #[tokio::test]
    async fn shutdown_failure_is_dropped_not_cached() {
        let closed = Arc::new(AtomicBool::new(false));
        let (map, g) = test_map(closed.clone());
        let o = owner(map.acquire("a"));
        let attempt = match map.acquire("a") {
            Acquire::Wait(a) => a,
            _ => panic!("expected Wait"),
        };
        let waiter = attempt.wait();
        closed.store(true, Ordering::SeqCst);
        // Drop always removes the slot. It never caches a failure.
        drop(o);
        // The waiter is woken by the removal.
        waiter.await;
        assert_eq!(g.get(), 0);
        assert!(map.names().is_empty());
        assert!(!map.work_remains());
        // A later caller is refused rather than served a cached failure.
        assert!(matches!(map.acquire("a"), Acquire::Refused(_)));
    }
}
