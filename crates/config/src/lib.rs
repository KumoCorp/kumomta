use anyhow::Context;
use mlua::{FromLuaMulti, Lua, RegistryKey, Table, ToLuaMulti, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref POLICY_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
    static ref FUNCS: Mutex<Vec<RegisterFunc>> = Mutex::new(vec![]);
    static ref POOL: Mutex<Pool> = Mutex::new(Pool::new());
    static ref LUA_LOAD_COUNT: metrics::Counter = {
        metrics::describe_counter!(
            "lua_load_count",
            "how many times the policy lua script has been \
             loaded into a new context");
        metrics::register_counter!("lua_load_count")
    };
    static ref LUA_COUNT: metrics::Gauge = {
        metrics::describe_gauge!(
            "lua_count", "the number of lua contexts currently alive");
        metrics::register_gauge!("lua_count")
    };
    static ref LUA_SPARE_COUNT: metrics::Gauge = {
        metrics::describe_gauge!(
            "lua_spare_count",
            "the number of lua contexts available for reuse in the pool");
        metrics::register_gauge!("lua_spare_count")
    };
}

/// Maximum age of a lua context before we release it
const MAX_AGE: Duration = Duration::from_secs(300);
/// Maximum number of uses of a given lua context before we release it
const MAX_USE: usize = 1024;
/// Maximum number of spare lua contexts to maintain in the pool
const MAX_SPARE: usize = 128;

pub type RegisterFunc = fn(&Lua) -> anyhow::Result<()>;

#[derive(Default)]
struct Pool {
    pool: VecDeque<LuaConfigInner>,
}

impl Pool {
    pub fn new() -> Self {
        std::thread::Builder::new()
            .name("config idler".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(30));
                POOL.lock().unwrap().expire();
            })
            .expect("create config idler thread");
        Self::default()
    }

    pub fn expire(&mut self) {
        let len_before = self.pool.len();
        self.pool.retain(|inner| inner.created.elapsed() < MAX_AGE);
        let len_after = self.pool.len();
        let diff = len_before - len_after;
        if diff > 0 {
            LUA_SPARE_COUNT.decrement(diff as f64);
        }
    }

    pub fn get(&mut self) -> Option<LuaConfigInner> {
        loop {
            let mut item = self.pool.pop_front()?;
            LUA_SPARE_COUNT.decrement(1.);
            if item.created.elapsed() > MAX_AGE {
                continue;
            }
            item.use_count += 1;
            return Some(item);
        }
    }

    pub fn put(&mut self, config: LuaConfigInner) {
        if self.pool.len() + 1 > MAX_SPARE {
            return;
        }
        if config.created.elapsed() > MAX_AGE || config.use_count + 1 > MAX_USE {
            return;
        }
        self.pool.push_back(config);
        LUA_SPARE_COUNT.increment(1.);
    }
}

#[derive(Debug)]
struct LuaConfigInner {
    lua: Lua,
    created: Instant,
    use_count: usize,
}

impl Drop for LuaConfigInner {
    fn drop(&mut self) {
        LUA_COUNT.decrement(1.);
    }
}

#[derive(Debug)]
pub struct LuaConfig {
    inner: Option<LuaConfigInner>,
}

impl Drop for LuaConfig {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            POOL.lock().unwrap().put(inner);
        }
    }
}

pub async fn set_policy_path(path: PathBuf) -> anyhow::Result<()> {
    POLICY_FILE.lock().unwrap().replace(path);
    load_config().await?;
    Ok(())
}

fn get_policy_path() -> Option<PathBuf> {
    POLICY_FILE.lock().unwrap().clone()
}

fn get_funcs() -> Vec<RegisterFunc> {
    FUNCS.lock().unwrap().clone()
}

pub async fn load_config() -> anyhow::Result<LuaConfig> {
    if let Some(inner) = POOL.lock().unwrap().get() {
        return Ok(LuaConfig { inner: Some(inner) });
    }

    LUA_LOAD_COUNT.increment(1);
    let lua = Lua::new();
    let created = Instant::now();

    {
        let globals = lua.globals();
        let package: Table = globals.get("package")?;
        let package_path: String = package.get("path")?;
        let mut path_array: Vec<String> = package_path.split(";").map(|s| s.to_owned()).collect();

        fn prefix_path(array: &mut Vec<String>, path: &str) {
            array.insert(0, format!("{}/?.lua", path));
            array.insert(1, format!("{}/?/init.lua", path));
        }

        prefix_path(&mut path_array, "/opt/kumomta/etc/policy");
        prefix_path(&mut path_array, "/opt/kumomta/share");

        #[cfg(debug_assertions)]
        prefix_path(&mut path_array, "assets");

        package.set("path", path_array.join(";"))?;
    }

    for func in get_funcs() {
        (func)(&lua)?;
    }

    if let Some(policy) = get_policy_path() {
        let code = tokio::fs::read_to_string(&policy)
            .await
            .with_context(|| format!("reading policy file {policy:?}"))?;

        let func = {
            let chunk = lua.load(&code);
            let chunk = chunk.set_name(policy.to_string_lossy())?;
            chunk.into_function()?
        };

        func.call_async(()).await?;
    }
    LUA_COUNT.increment(1.);

    Ok(LuaConfig {
        inner: Some(LuaConfigInner {
            lua,
            created,
            use_count: 1,
        }),
    })
}

pub fn register(func: RegisterFunc) {
    FUNCS.lock().unwrap().push(func);
}

impl LuaConfig {
    /// Call a callback registered via `on`.
    pub async fn async_call_callback<
        'lua,
        S: AsRef<str>,
        A: ToLuaMulti<'lua> + Clone,
        R: FromLuaMulti<'lua> + Default,
    >(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<R> {
        let name = name.as_ref();
        let decorated_name = format!("kumomta-on-{}", name);
        match self
            .inner
            .as_mut()
            .unwrap()
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)
        {
            Ok(func) => Ok(func.call_async(args.clone()).await?),
            _ => Ok(R::default()),
        }
    }

    pub fn remove_registry_value(&mut self, value: RegistryKey) -> anyhow::Result<()> {
        Ok(self
            .inner
            .as_mut()
            .unwrap()
            .lua
            .remove_registry_value(value)?)
    }

    /// Call a constructor registered via `on`. Returns a registry key that can be
    /// used to reference the returned value again later on this same Lua instance
    pub async fn async_call_ctor<'lua, S: AsRef<str>, A: ToLuaMulti<'lua> + Clone>(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<RegistryKey> {
        let name = name.as_ref();
        let decorated_name = format!("kumomta-on-{}", name);

        let inner = self.inner.as_mut().unwrap();

        let func = inner
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)?;

        let value: Value = func.call_async(args.clone()).await?;
        drop(func);

        Ok(inner.lua.create_registry_value(value)?)
    }

    /// Operate on an object/value that was previously constructed via
    /// async_call_ctor.
    pub async fn with_registry_value<'lua, F, R, FUT>(
        &'lua mut self,
        value: &RegistryKey,
        func: F,
    ) -> anyhow::Result<R>
    where
        R: FromLuaMulti<'lua>,
        F: FnOnce(Value<'lua>) -> anyhow::Result<FUT>,
        FUT: std::future::Future<Output = anyhow::Result<R>> + 'lua,
    {
        let inner = self.inner.as_mut().unwrap();
        let value = inner.lua.registry_value(value)?;
        let future = (func)(value)?;
        future.await
    }

    /// Call a callback registered via `on`.
    #[allow(unused)]
    pub fn call_callback<
        'lua,
        S: AsRef<str>,
        A: ToLuaMulti<'lua> + Clone,
        R: FromLuaMulti<'lua> + Default,
    >(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<R> {
        let name = name.as_ref();
        let decorated_name = format!("kumomta-on-{}", name);
        match self
            .inner
            .as_mut()
            .unwrap()
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)
        {
            Ok(func) => Ok(func.call(args.clone())?),
            _ => Ok(R::default()),
        }
    }
}

pub fn get_or_create_module<'lua>(lua: &'lua Lua, name: &str) -> anyhow::Result<mlua::Table<'lua>> {
    let globals = lua.globals();
    let package: Table = globals.get("package")?;
    let loaded: Table = package.get("loaded")?;

    let module = loaded.get(name)?;
    match module {
        Value::Nil => {
            let module = lua.create_table()?;
            loaded.set(name, module.clone())?;
            Ok(module)
        }
        Value::Table(table) => Ok(table),
        wat => anyhow::bail!(
            "cannot register module {} as package.loaded.{} is already set to a value of type {}",
            name,
            name,
            wat.type_name()
        ),
    }
}

pub fn get_or_create_sub_module<'lua>(
    lua: &'lua Lua,
    name: &str,
) -> anyhow::Result<mlua::Table<'lua>> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    let sub = kumo_mod.get(name)?;
    match sub {
        Value::Nil => {
            let sub = lua.create_table()?;
            kumo_mod.set(name, sub.clone())?;
            Ok(sub)
        }
        Value::Table(sub) => Ok(sub),
        wat => anyhow::bail!(
            "cannot register module kumo.{name} as it is already set to a value of type {}",
            wat.type_name()
        ),
    }
}

/// Helper for mapping back to lua errors
pub fn any_err<E: std::fmt::Display>(err: E) -> mlua::Error {
    mlua::Error::external(format!("{err:#}"))
}
