use anyhow::Context;
use mlua::{FromLua, FromLuaMulti, Lua, LuaSerdeExt, RegistryKey, Table, ToLuaMulti, Value};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    static ref CALLBACK_ALLOWS_MULTIPLE: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

/// Maximum age of a lua context before we release it, in seconds
static MAX_AGE: AtomicUsize = AtomicUsize::new(300);
/// Maximum number of uses of a given lua context before we release it
static MAX_USE: AtomicUsize = AtomicUsize::new(1024);
/// Maximum number of spare lua contexts to maintain in the pool
static MAX_SPARE: AtomicUsize = AtomicUsize::new(8192);

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

pub fn set_max_use(max_use: usize) {
    MAX_USE.store(max_use, Ordering::Relaxed);
}

pub fn set_max_spare(max_spare: usize) {
    MAX_SPARE.store(max_spare, Ordering::Relaxed);
}

pub fn set_max_age(max_age: usize) {
    MAX_AGE.store(max_age, Ordering::Relaxed);
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
    fn set_current_event(&mut self, name: &str) -> mlua::Result<()> {
        self.inner
            .as_mut()
            .unwrap()
            .lua
            .globals()
            .set("_KUMO_CURRENT_EVENT", name.to_string())
    }

    pub async fn async_call_callback<
        'lua,
        A: ToLuaMulti<'lua> + Clone,
        R: FromLuaMulti<'lua> + Default,
    >(
        &'lua mut self,
        sig: &CallbackSignature<'lua, A, R>,
        args: A,
    ) -> anyhow::Result<R> {
        let name = sig.name();
        let decorated_name = sig.decorated_name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();

        if sig.allow_multiple() {
            return match lua
                .lua
                .named_registry_value::<_, mlua::Value>(&decorated_name)?
            {
                Value::Table(tbl) => {
                    for func in tbl.sequence_values::<mlua::Function>() {
                        let func = func?;
                        let result: mlua::MultiValue = func.call_async(args.clone()).await?;
                        if result.is_empty() {
                            // Continue with other handlers
                            continue;
                        }
                        let result = R::from_lua_multi(result, &lua.lua)?;
                        return Ok(result);
                    }
                    Ok(R::default())
                }
                _ => Ok(R::default()),
            };
        }

        match lua
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)
        {
            Ok(func) => Ok(func.call_async(args.clone()).await?),
            _ => Ok(R::default()),
        }
    }

    pub async fn async_call_callback_non_default<
        'lua,
        A: ToLuaMulti<'lua> + Clone,
        R: FromLuaMulti<'lua>,
    >(
        &'lua mut self,
        sig: &CallbackSignature<'lua, A, R>,
        args: A,
    ) -> anyhow::Result<R> {
        let name = sig.name();
        let decorated_name = sig.decorated_name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();

        if sig.allow_multiple() {
            match lua
                .lua
                .named_registry_value::<_, mlua::Value>(&decorated_name)?
            {
                Value::Table(tbl) => {
                    for func in tbl.sequence_values::<mlua::Function>() {
                        let func = func?;
                        let result: mlua::MultiValue = func.call_async(args.clone()).await?;
                        if result.is_empty() {
                            // Continue with other handlers
                            continue;
                        }
                        let result = R::from_lua_multi(result, &lua.lua)?;
                        return Ok(result);
                    }
                }
                _ => {}
            };
            anyhow::bail!("invalid return type for {name} event");
        }

        match lua
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)
        {
            Ok(func) => Ok(func.call_async(args.clone()).await?),
            _ => anyhow::bail!("invalid return type for {name} event"),
        }
    }

    pub async fn async_call_callback_non_default_opt<
        'lua,
        A: ToLuaMulti<'lua> + Clone,
        R: FromLua<'lua>,
    >(
        &'lua mut self,
        sig: &CallbackSignature<'lua, A, Option<R>>,
        args: A,
    ) -> anyhow::Result<Option<R>> {
        let name = sig.name();
        let decorated_name = sig.decorated_name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();

        if sig.allow_multiple() {
            return match lua
                .lua
                .named_registry_value::<_, mlua::Value>(&decorated_name)?
            {
                Value::Table(tbl) => {
                    for func in tbl.sequence_values::<mlua::Function>() {
                        let func = func?;
                        let result: mlua::MultiValue = func.call_async(args.clone()).await?;
                        if result.is_empty() {
                            // Continue with other handlers
                            continue;
                        }
                        let result = R::from_lua_multi(result, &lua.lua)?;
                        return Ok(Some(result));
                    }
                    Ok(None)
                }
                _ => Ok(None),
            };
        }

        let opt_func: mlua::Value = lua.lua.named_registry_value(&decorated_name)?;

        match opt_func {
            Value::Nil => Ok(None),
            Value::Function(func) => {
                let value: Value = func.call_async(args.clone()).await?;

                match value {
                    Value::Nil => Ok(None),
                    value => {
                        let result = R::from_lua(value, &lua.lua)?;
                        Ok(Some(result))
                    }
                }
            }
            _ => anyhow::bail!("invalid return type for {name} event"),
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
    pub async fn async_call_ctor<'lua, A: ToLuaMulti<'lua> + Clone>(
        &'lua mut self,
        sig: &CallbackSignature<'lua, A, Value<'lua>>,
        args: A,
    ) -> anyhow::Result<RegistryKey> {
        let name = sig.name();
        anyhow::ensure!(
            !sig.allow_multiple(),
            "ctor event signature for {name} is defined as allow_multiple, which is not supported"
        );

        let decorated_name = sig.decorated_name();
        self.set_current_event(name)?;

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

/// Convert from a lua value to a deserializable type,
/// with a slightly more helpful error message in case of failure.
pub fn from_lua_value<'lua, R>(lua: &'lua Lua, value: mlua::Value<'lua>) -> mlua::Result<R>
where
    R: serde::Deserialize<'lua>,
{
    let value_cloned = value.clone();
    lua.from_value(value).map_err(|err| {
        let mut serializer = serde_json::Serializer::new(Vec::new());
        let serialized = match value_cloned.serialize(&mut serializer) {
            Ok(_) => String::from_utf8_lossy(&serializer.into_inner()).to_string(),
            Err(err) => format!("<unable to encode as json: {err:#}>"),
        };
        // Unconditionally log this here; there are a number of contexts where
        // the error might get logged to the delivery logs instead of showing
        // up in the face of the administrator, but this class of error implies
        // a serious issue with the configuration
        tracing::error!("{err:#}, while processing {serialized}");
        mlua::Error::external(format!("{err:#}, while processing {serialized}"))
    })
}

/// CallbackSignature is a bit sugar to aid with statically typing event callback
/// function invocation.
///
/// The idea is that you declare a signature instance that is typed
/// with its argument tuple (A), and its return type tuple (R).
///
/// The signature instance can then be used to invoke the callback by name.
///
/// The register method allows pre-registering events so that `kumo.on`
/// can reason about them better.  The main function enabled by this is
/// `allow_multiple`; when that is set to true, `kumo.on` will allow
/// recording multiple callback instances, calling them in sequence
/// until one of them returns a value.
pub struct CallbackSignature<'lua, A, R>
where
    A: ToLuaMulti<'lua>,
    R: FromLuaMulti<'lua>,
{
    marker: std::marker::PhantomData<&'lua (A, R)>,
    allow_multiple: bool,
    name: Cow<'static, str>,
}

impl<'lua, A, R> CallbackSignature<'lua, A, R>
where
    A: ToLuaMulti<'lua>,
    R: FromLuaMulti<'lua>,
{
    pub fn new<S: Into<Cow<'static, str>>>(name: S) -> Self {
        let name = name.into();

        Self {
            marker: std::marker::PhantomData,
            allow_multiple: false,
            name,
        }
    }

    pub fn new_with_multiple<S: Into<Cow<'static, str>>>(name: S) -> Self {
        let name = name.into();

        Self {
            marker: std::marker::PhantomData,
            allow_multiple: true,
            name,
        }
    }

    pub fn register(&self) {
        if self.allow_multiple {
            CALLBACK_ALLOWS_MULTIPLE
                .lock()
                .unwrap()
                .insert(self.name.to_string());
        }
    }

    /// Return true if this signature allows multiple instances to be registered
    /// and called.
    pub fn allow_multiple(&self) -> bool {
        self.allow_multiple
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn decorated_name(&self) -> String {
        decorate_callback_name(&self.name)
    }
}

pub fn does_callback_allow_multiple(name: &str) -> bool {
    CALLBACK_ALLOWS_MULTIPLE.lock().unwrap().contains(name)
}

pub fn decorate_callback_name(name: &str) -> String {
    format!("kumomta-on-{name}")
}
