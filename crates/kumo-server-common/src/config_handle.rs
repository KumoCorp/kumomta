use std::cell::{Ref, RefCell};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Inner<T> {
    config: Mutex<T>,
    generation: AtomicUsize,
}

impl<T: Debug> Debug for Inner<T> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("Inner")
            .field("config", &self.config)
            .field("generation", &self.generation)
            .finish()
    }
}

/// ConfigHandle allows sharing a configuration value of some type T
/// without requiring full Mutex around every read operation.
///
/// The handle holds a local copy along with a shared reference to
/// the "upstream" value and a generation counter.
///
/// The update method will lock and update the upstream value, bumping
/// the generation counter in the process.
///
/// The borrow method will compare its local snapshot of the generation
/// counter with that of the upstream using a non-blocking atomic load.
/// If the generation counters match then the local config value is
/// current and we can simply return a reference.  Otherwise, we'll lock
/// the upstream and copy and update the config and generation counter.
///
/// This approach defers locking until we know that the config has
/// changed.
#[derive(Clone)]
pub struct ConfigHandle<T: Clone> {
    inner: Arc<Inner<T>>,
    config: RefCell<T>,
    generation: RefCell<usize>,
}

impl<T: Clone + Debug> Debug for ConfigHandle<T> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("ConfigHandle")
            .field("inner", &self.inner)
            .field("config", &self.config)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<T: Clone> ConfigHandle<T> {
    /// If the generation counts are different (eg: someone called `update`),
    /// then lock and copy the updated value.
    /// Otherwise we are in the fast path and do nothing.
    fn update_local_copy(&self) {
        let upstream_generation = self.inner.generation.load(Ordering::SeqCst);
        if upstream_generation == *self.generation.borrow() {
            // Already current
            return;
        }

        let upstream = self.inner.config.lock().unwrap();
        *self.config.borrow_mut() = upstream.clone();
        // Read the generation again, as the lock may have
        // waited and the generation may have changed again
        *self.generation.borrow_mut() = self.inner.generation.load(Ordering::SeqCst);
    }

    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(Inner {
                config: Mutex::new(value.clone()),
                generation: AtomicUsize::new(0),
            }),
            config: RefCell::new(value),
            generation: RefCell::new(0),
        }
    }

    /// Updates the upstream, shared value.
    /// Other config handles will notice the change when borrow() is subsequently called.
    pub fn update(&self, new_value: T) -> usize {
        let mut upstream = self.inner.config.lock().unwrap();

        *upstream = new_value;
        self.inner.generation.fetch_add(1, Ordering::SeqCst);

        *self.config.borrow_mut() = upstream.clone();
        let generation = self.inner.generation.load(Ordering::SeqCst);
        *self.generation.borrow_mut() = generation;
        generation
    }

    /// Borrows the local copy of the config for read.
    /// The local copy will be updated from the upstream if necessary.
    pub fn borrow(&self) -> Ref<'_, T> {
        self.update_local_copy();
        self.config.borrow()
    }
}
