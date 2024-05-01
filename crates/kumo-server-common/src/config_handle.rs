use arc_swap::{ArcSwap, Guard};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// ConfigHandle allows sharing a configuration value of some type T
/// without requiring full Mutex around every read operation.
///
/// The update method update the upstream value, bumping
/// the generation counter in the process.
///
/// The borrow method will return a Guard referencing the value
/// without requiring a Mutex lock operation.
#[derive(Clone)]
pub struct ConfigHandle<T: Clone + Send> {
    inner: Arc<Inner<T>>,
}

struct Inner<T: Clone + Send> {
    value: ArcSwap<T>,
    generation: AtomicUsize,
}

impl<T: Clone + Send + Debug> Debug for ConfigHandle<T> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("ConfigHandle")
            .field("value", &self.inner.value)
            .field("generation", &self.inner.generation)
            .finish()
    }
}

impl<T: Clone + Send> ConfigHandle<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(Inner {
                value: ArcSwap::from_pointee(value),
                generation: AtomicUsize::new(0),
            }),
        }
    }

    /// Updates the upstream, shared value.
    /// Other config handles will notice the change when borrow() is subsequently called.
    pub fn update(&self, new_value: T) -> usize {
        self.inner.value.swap(Arc::new(new_value));
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        generation
    }

    /// Borrows the local copy of the config for read.
    /// The local copy will be updated from the upstream if necessary.
    pub fn borrow(&self) -> Guard<Arc<T>> {
        self.inner.value.load()
    }
}
