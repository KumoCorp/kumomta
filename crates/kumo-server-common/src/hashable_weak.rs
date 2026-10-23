use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

/// `Weak<T>` newtype that hashes and compares by the underlying
/// allocation address, so it can be stored in a `HashSet` or used
/// as a `HashMap` key.
///
/// Equality uses `Weak::ptr_eq`. The `ptr_eq` semantics are
/// documented as unspecified once the original allocation has been
/// dropped, so callers should periodically prune dead entries (via
/// `Weak::upgrade` returning `None`) to keep the set hygienic. With
/// timely pruning, the only observable consequence of a stale
/// entry colliding with a new live entry is that the live entry
/// replaces the stale one on insert — which is the desired outcome
/// anyway.
pub struct HashableWeak<T: ?Sized>(pub Weak<T>);

impl<T: ?Sized> HashableWeak<T> {
    pub fn new(arc: &Arc<T>) -> Self {
        Self(Arc::downgrade(arc))
    }
}

impl<T> Hash for HashableWeak<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0.as_ptr() as usize).hash(state)
    }
}

impl<T: ?Sized> PartialEq for HashableWeak<T> {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl<T: ?Sized> Eq for HashableWeak<T> {}

impl<T: ?Sized> Clone for HashableWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn dedup_by_pointer_identity() {
        let a = Arc::new(42u32);
        let b = Arc::new(42u32);
        let mut set: HashSet<HashableWeak<u32>> = HashSet::new();
        set.insert(HashableWeak::new(&a));
        set.insert(HashableWeak::new(&a));
        set.insert(HashableWeak::new(&b));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn prune_via_upgrade() {
        let a = Arc::new(42u32);
        let mut set: HashSet<HashableWeak<u32>> = HashSet::new();
        set.insert(HashableWeak::new(&a));
        drop(a);
        set.retain(|hw| hw.0.upgrade().is_some());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn iter_upgrade_collects_live() {
        let a = Arc::new(1u32);
        let b = Arc::new(2u32);
        let mut set: HashSet<HashableWeak<u32>> = HashSet::new();
        set.insert(HashableWeak::new(&a));
        set.insert(HashableWeak::new(&b));
        drop(a);
        let live: Vec<u32> = set
            .iter()
            .filter_map(|hw| hw.0.upgrade().map(|v| *v))
            .collect();
        assert_eq!(live, vec![2]);
    }
}
