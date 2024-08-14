use crate::{LuaConfig, LuaConfigInner};
use parking_lot::FairMutex as Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

lazy_static::lazy_static! {
    static ref POOL: Mutex<Pool> = Mutex::new(Pool::new());
    static ref LUA_SPARE_COUNT: metrics::Gauge = {
        metrics::describe_gauge!(
            "lua_spare_count",
            "the number of lua contexts available for reuse in the pool");
        metrics::gauge!("lua_spare_count")
    };
}

/// Maximum age of a lua context before we release it, in seconds
static MAX_AGE: AtomicUsize = AtomicUsize::new(300);
/// Maximum number of uses of a given lua context before we release it
static MAX_USE: AtomicUsize = AtomicUsize::new(1024);
/// Maximum number of spare lua contexts to maintain in the pool
static MAX_SPARE: AtomicUsize = AtomicUsize::new(8192);

pub fn set_max_use(max_use: usize) {
    MAX_USE.store(max_use, Ordering::Relaxed);
}

pub fn set_max_spare(max_spare: usize) {
    MAX_SPARE.store(max_spare, Ordering::Relaxed);
}

pub fn set_max_age(max_age: usize) {
    MAX_AGE.store(max_age, Ordering::Relaxed);
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
        let max_age = Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64);
        self.pool.retain(|inner| inner.created.elapsed() < max_age);
        let len_after = self.pool.len();
        let diff = len_before - len_after;
        if diff > 0 {
            LUA_SPARE_COUNT.decrement(diff as f64);
        }
    }

    pub fn get(&mut self) -> Option<LuaConfigInner> {
        let max_age = Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64);
        loop {
            let mut item = self.pool.pop_front()?;
            LUA_SPARE_COUNT.decrement(1.);
            if item.created.elapsed() > max_age {
                continue;
            }
            item.use_count += 1;
            return Some(item);
        }
    }

    pub fn put(&mut self, config: LuaConfigInner) {
        if self.pool.len() + 1 > MAX_SPARE.load(Ordering::Relaxed) {
            return;
        }
        if config.created.elapsed() > Duration::from_secs(MAX_AGE.load(Ordering::Relaxed) as u64)
            || config.use_count + 1 > MAX_USE.load(Ordering::Relaxed)
        {
            return;
        }
        self.pool.push_back(config);
        LUA_SPARE_COUNT.increment(1.);
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
