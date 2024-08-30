use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

/// This trait enables having prunable and non-pruning value types
/// in the CounterRegistry.
pub trait AtomicCounterEntry: Send + Sync {
    /// resolve this entry to an AtomicCounter instance
    fn resolve(&self) -> Option<AtomicCounter>;
    /// Given a new strong AtomicCounter reference, return Self
    /// suitable for storing in the counter registry
    fn make_storable(strong: &AtomicCounter) -> Self;
    /// Indicate whether this type of value requires pruning the
    /// containing counter registry
    fn needs_pruning() -> bool;
}

#[derive(Clone)]
pub struct WeakAtomicCounter(Weak<AtomicUsize>);

impl WeakAtomicCounter {
    pub fn upgrade(&self) -> Option<AtomicCounter> {
        Some(AtomicCounter(self.0.upgrade()?))
    }
}

/// WeakAtomicCounter stores values as weak references and thus
/// requires pruning.
impl AtomicCounterEntry for WeakAtomicCounter {
    fn resolve(&self) -> Option<AtomicCounter> {
        self.upgrade()
    }

    fn make_storable(strong: &AtomicCounter) -> WeakAtomicCounter {
        strong.weak()
    }

    fn needs_pruning() -> bool {
        true
    }
}

#[derive(Clone)]
pub struct AtomicCounter(Arc<AtomicUsize>);

/// AtomicCounter is a direct store of the underlying counter value.
/// No pruning is required for this type of value.
impl AtomicCounterEntry for AtomicCounter {
    fn resolve(&self) -> Option<AtomicCounter> {
        Some(self.clone())
    }

    fn make_storable(strong: &AtomicCounter) -> AtomicCounter {
        strong.clone()
    }

    fn needs_pruning() -> bool {
        false
    }
}

impl std::fmt::Debug for AtomicCounter {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("AtomicCounter").finish()
    }
}

impl AtomicCounter {
    pub fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }

    pub fn weak(&self) -> WeakAtomicCounter {
        WeakAtomicCounter(Arc::downgrade(&self.0))
    }

    #[inline]
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn set(&self, v: usize) {
        self.0.store(v, Ordering::Relaxed)
    }

    #[inline]
    pub fn inc(&self) {
        self.inc_by(1);
    }

    #[inline]
    pub fn inc_by(&self, n: usize) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    #[inline]
    pub fn dec(&self) {
        self.sub(1);
    }

    #[inline]
    pub fn sub(&self, n: usize) {
        self.0.fetch_sub(n, Ordering::Relaxed);
    }
}

impl Default for AtomicCounter {
    fn default() -> Self {
        Self::new()
    }
}
