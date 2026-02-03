use crate::{get_current_epoch, LuaConfig, LuaConfigInner};
use kumo_prometheus::declare_metric;
use parking_lot::FairMutex as Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| Mutex::new(Pool::new()));

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
                POOL.lock().expire();
            })
            .expect("create config idler thread");
        Self::default()
    }

    pub fn expire(&mut self) {
        let len_before = self.pool.len();
        let epoch = get_current_epoch();
        let max_age = Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64);
        self.pool
            .retain(|inner| inner.created.elapsed() < max_age && inner.epoch == epoch);
        let len_after = self.pool.len();
        let diff = len_before - len_after;
        if diff > 0 {
            LUA_SPARE_COUNT.sub(diff as i64);
        }
    }

    pub fn get(&mut self) -> Option<LuaConfigInner> {
        let max_age = Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64);
        loop {
            let mut item = self.pool.pop_front()?;
            LUA_SPARE_COUNT.dec();
            if item.created.elapsed() > max_age || item.epoch != get_current_epoch() {
                continue;
            }
            item.use_count += 1;
            return Some(item);
        }
    }

    pub fn put(&mut self, config: LuaConfigInner) {
        let epoch = get_current_epoch();
        if config.epoch != epoch {
            // Stale; let it drop
            return;
        }
        if self.pool.len() + 1 > MAX_SPARE.load(Ordering::Relaxed) {
            return;
        }
        if config.created.elapsed() > Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64)
            || config.use_count + 1 > MAX_USE.load(Ordering::Relaxed)
        {
            return;
        }
        let prob = GC_ON_PUT.load(Ordering::Relaxed);
        if prob != 0 {
            let chance = (rand::random::<f32>() * 100.) as usize;
            if chance <= prob {
                if let Err(err) = config.lua.gc_collect() {
                    tracing::error!("Error during gc: {err:#}");
                    return;
                }
            }
        }

        self.pool.push_back(config);
        LUA_SPARE_COUNT.inc();
    }
}

pub(crate) fn pool_get() -> Option<LuaConfig> {
    POOL.lock()
        .get()
        .map(|inner| LuaConfig { inner: Some(inner) })
}

pub(crate) fn pool_put(config: LuaConfigInner) {
    POOL.lock().put(config);
}
