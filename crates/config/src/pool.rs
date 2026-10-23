use crate::{get_current_epoch, LuaConfig, LuaConfigInner};
use kumo_prometheus::declare_metric;
use parking_lot::FairMutex as Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::LazyLock;
use std::time::Duration;

static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| Mutex::new(Pool::new()));

static DROPPER: LazyLock<Sender<LuaConfigInner>> = LazyLock::new(|| {
    let (tx, rx) = channel::<LuaConfigInner>();
    std::thread::Builder::new()
        .name("lua context dropper".to_string())
        .spawn(move || {
            for context in rx {
                drop(context);
            }
        })
        .expect("create lua context dropper thread");
    tx
});

declare_metric! {
/// the number of lua contexts available for reuse in the pool
static LUA_SPARE_COUNT: IntGauge("lua_spare_count");
}

/// Maximum age of a lua context before we release it, in seconds
static MAX_AGE: AtomicUsize = AtomicUsize::new(300);
/// Maximum number of uses of a given lua context before we release it
static MAX_USE: AtomicUsize = AtomicUsize::new(1024);
/// Maximum number of spare lua contexts to maintain in the pool
static MAX_SPARE: AtomicUsize = AtomicUsize::new(8192);
static GC_ON_PUT: AtomicUsize = AtomicUsize::new(0);

pub fn set_max_use(max_use: usize) {
    MAX_USE.store(max_use, Ordering::Relaxed);
}

pub fn set_max_spare(max_spare: usize) {
    MAX_SPARE.store(max_spare, Ordering::Relaxed);
}

pub fn set_max_age(max_age: usize) {
    MAX_AGE.store(max_age, Ordering::Relaxed);
}

/// Set the gc on put percentage chance, in the range 0-100
pub fn set_gc_on_put(v: u8) {
    GC_ON_PUT.store(v as usize, Ordering::Relaxed);
}

fn max_age() -> Duration {
    Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64)
}

/// Retire a context by handing it to the dropper thread, which closes it
/// asynchronously instead of making the caller pay for the close.
fn discard(config: LuaConfigInner) {
    if let Err(err) = DROPPER.send(config) {
        drop(err.0);
    }
}

fn discard_all(contexts: Vec<LuaConfigInner>) {
    for context in contexts {
        discard(context);
    }
}

#[derive(Default)]
pub(crate) struct Pool {
    pool: VecDeque<LuaConfigInner>,
}

impl Pool {
    pub fn new() -> Self {
        std::thread::Builder::new()
            .name("config idler".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(30));
                let expired = POOL.lock().expire();
                discard_all(expired);
            })
            .expect("create config idler thread");
        Self::default()
    }

    /// Remove the contexts that have aged out or belong to an earlier
    /// epoch, returning them for the caller to discard after releasing
    /// the lock.
    pub fn expire(&mut self) -> Vec<LuaConfigInner> {
        let epoch = get_current_epoch();
        let max_age = max_age();
        let (keep, expired): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pool)
            .into_iter()
            .partition(|inner| inner.created.elapsed() < max_age && inner.epoch == epoch);
        self.pool = VecDeque::from(keep);
        if !expired.is_empty() {
            LUA_SPARE_COUNT.sub(expired.len() as i64);
        }
        expired
    }

    /// Returns a reusable context, if there is one, plus any stale contexts
    /// found while looking. The caller must discard the stale contexts
    /// after releasing the pool lock.
    pub fn get(&mut self) -> (Option<LuaConfigInner>, Vec<LuaConfigInner>) {
        let max_age = max_age();
        let mut stale = vec![];
        while let Some(mut item) = self.pool.pop_front() {
            LUA_SPARE_COUNT.dec();
            if item.created.elapsed() > max_age || item.epoch != get_current_epoch() {
                stale.push(item);
                continue;
            }
            item.use_count += 1;
            return (Some(item), stale);
        }
        (None, stale)
    }

    /// Put a context back into the pool, or return it to the caller
    /// if the pool is already at capacity.  The caller must discard the
    /// returned context.
    pub fn put(&mut self, config: LuaConfigInner) -> Option<LuaConfigInner> {
        if self.pool.len() + 1 > MAX_SPARE.load(Ordering::Relaxed) {
            return Some(config);
        }
        self.pool.push_back(config);
        LUA_SPARE_COUNT.inc();
        None
    }
}

pub(crate) fn pool_get() -> Option<LuaConfig> {
    let (item, stale) = POOL.lock().get();
    discard_all(stale);
    item.map(|inner| LuaConfig { inner: Some(inner) })
}

pub(crate) fn pool_put(config: LuaConfigInner) {
    let reusable = config.created.elapsed() <= max_age()
        && config.use_count + 1 <= MAX_USE.load(Ordering::Relaxed);
    if !reusable {
        discard(config);
        return;
    }

    // gc can be slow, so it runs before the lock is taken.
    let prob = GC_ON_PUT.load(Ordering::Relaxed);
    if prob != 0 {
        let chance = (rand::random::<f32>() * 100.) as usize;
        if chance <= prob {
            if let Err(err) = config.lua.gc_collect() {
                tracing::error!("Error during gc: {err:#}");
                discard(config);
                return;
            }
        }
    }

    let mut pool = POOL.lock();
    // The optional gc pass before this lock can take long enough for another
    // thread to bump the epoch while it runs. This recheck catches a context
    // that went stale during that gc.
    if config.epoch != get_current_epoch() {
        drop(pool);
        discard(config);
        return;
    }
    if let Some(config) = pool.put(config) {
        drop(pool);
        discard(config);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::epoch::bump_current_epoch;
    use mlua::Lua;
    use std::time::Instant;

    fn new_inner() -> LuaConfigInner {
        LuaConfigInner {
            lua: Lua::new(),
            created: Instant::now(),
            use_count: 1,
            epoch: get_current_epoch(),
        }
    }

    #[test]
    fn get_hands_back_contexts_from_an_earlier_epoch() {
        let mut pool = Pool::default();
        assert!(pool.put(new_inner()).is_none());
        assert!(pool.put(new_inner()).is_none());

        bump_current_epoch();

        let (item, stale) = pool.get();
        assert!(item.is_none());
        k9::assert_equal!(stale.len(), 2);
        k9::assert_equal!(pool.pool.len(), 0);
    }

    #[test]
    fn expire_hands_back_contexts_from_an_earlier_epoch() {
        let mut pool = Pool::default();
        assert!(pool.put(new_inner()).is_none());

        bump_current_epoch();
        assert!(pool.put(new_inner()).is_none());

        let expired = pool.expire();
        k9::assert_equal!(expired.len(), 1);
        k9::assert_equal!(pool.pool.len(), 1);
    }

    /// Guard that restores the global MAX_SPARE on drop. Tests that lower
    /// MAX_SPARE for a capacity check use it, because tests in the same
    /// process share the global.
    struct RestoreMaxSpare(usize);

    impl Drop for RestoreMaxSpare {
        fn drop(&mut self) {
            set_max_spare(self.0);
        }
    }

    #[test]
    fn put_hands_back_the_context_when_the_pool_is_full() {
        let _restore = RestoreMaxSpare(MAX_SPARE.load(Ordering::Relaxed));
        set_max_spare(1);
        let mut pool = Pool::default();
        assert!(pool.put(new_inner()).is_none());
        assert!(pool.put(new_inner()).is_some());
    }
}
