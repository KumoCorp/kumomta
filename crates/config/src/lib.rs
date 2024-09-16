use crate::pool::{pool_get, pool_put};
pub use crate::pool::{set_gc_on_put, set_max_age, set_max_spare, set_max_use};
use anyhow::Context;
use mlua::{FromLua, FromLuaMulti, IntoLuaMulti, Lua, LuaSerdeExt, RegistryKey, Table, Value};
use once_cell::sync::Lazy;
use parking_lot::FairMutex as Mutex;
use prometheus::{CounterVec, HistogramTimer, HistogramVec};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub mod epoch;
mod pool;

lazy_static::lazy_static! {
    static ref POLICY_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
    static ref FUNCS: Mutex<Vec<RegisterFunc>> = Mutex::new(vec![]);
    static ref LUA_LOAD_COUNT: metrics::Counter = {
        metrics::describe_counter!(
            "lua_load_count",
            "how many times the policy lua script has been \
             loaded into a new context");
        metrics::counter!("lua_load_count")
    };
    static ref LUA_COUNT: metrics::Gauge = {
        metrics::describe_gauge!(
            "lua_count", "the number of lua contexts currently alive");
        metrics::gauge!("lua_count")
    };
    static ref CALLBACK_ALLOWS_MULTIPLE: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

pub static VALIDATE_ONLY: AtomicBool = AtomicBool::new(false);
pub static VALIDATION_FAILED: AtomicBool = AtomicBool::new(false);
static LATENCY_HIST: Lazy<HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec!(
        "lua_event_latency",
        "how long a given lua event callback took",
        &["event"]
    )
    .unwrap()
});
static EVENT_STARTED_COUNT: Lazy<CounterVec> = Lazy::new(|| {
    prometheus::register_counter_vec!(
        "lua_event_started",
        "Incremented each time we start to call a lua event callback. Use lua_event_latency_count to track completed events",
        &["event"]
    )
    .unwrap()
});

pub type RegisterFunc = fn(&Lua) -> anyhow::Result<()>;

fn latency_timer(label: &str) -> HistogramTimer {
    EVENT_STARTED_COUNT
        .get_metric_with_label_values(&[label])
        .expect("to get counter")
        .inc();
    LATENCY_HIST
        .get_metric_with_label_values(&[label])
        .expect("to get histo")
        .start_timer()
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
            pool_put(inner);
        }
    }
}

pub async fn set_policy_path(path: PathBuf) -> anyhow::Result<()> {
    POLICY_FILE.lock().replace(path);
    load_config().await?;
    Ok(())
}

fn get_policy_path() -> Option<PathBuf> {
    POLICY_FILE.lock().clone()
}

fn get_funcs() -> Vec<RegisterFunc> {
    FUNCS.lock().clone()
}
pub fn is_validating() -> bool {
    VALIDATE_ONLY.load(Ordering::Relaxed)
}

pub fn validation_failed() -> bool {
    VALIDATION_FAILED.load(Ordering::Relaxed)
}

pub fn set_validation_failed() {
    VALIDATION_FAILED.store(true, Ordering::Relaxed)
}

pub async fn load_config() -> anyhow::Result<LuaConfig> {
    if let Some(pool) = pool_get() {
        return Ok(pool);
    }

    LUA_LOAD_COUNT.increment(1);
    let lua = Lua::new();
    let created = Instant::now();

    {
        let globals = lua.globals();

        if is_validating() {
            globals.set("_VALIDATING_CONFIG", true)?;
        }

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
            let chunk = chunk.set_name(policy.to_string_lossy());
            chunk.into_function()?
        };

        let _timer = latency_timer("context-creation");
        func.call_async::<_, ()>(()).await?;
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
    FUNCS.lock().push(func);
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

    /// Intended to be used together with kumo.spawn_task
    pub async fn convert_args_and_call_callback<'lua, A: Serialize>(
        &'lua mut self,
        sig: &CallbackSignature<'lua, Value<'lua>, ()>,
        args: A,
    ) -> anyhow::Result<()> {
        let lua = self.inner.as_mut().unwrap();
        let args = lua.lua.to_value(&args)?;

        let name = sig.name();
        let decorated_name = sig.decorated_name();

        match lua
            .lua
            .named_registry_value::<mlua::Function>(&decorated_name)
        {
            Ok(func) => {
                let _timer = latency_timer(name);
                Ok(func.call_async(args).await?)
            }
            _ => anyhow::bail!("{name} has not been registered"),
        }
    }

    pub async fn async_call_callback<
        'lua,
        A: IntoLuaMulti<'lua> + Clone,
        R: FromLuaMulti<'lua> + Default,
    >(
        &'lua mut self,
        sig: &CallbackSignature<'lua, A, R>,
        args: A,
    ) -> anyhow::Result<R> {
        let name = sig.name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();
        async_call_callback(&lua.lua, sig, args).await
    }

    pub async fn async_call_callback_non_default<
        'lua,
        A: IntoLuaMulti<'lua> + Clone,
        R: FromLuaMulti<'lua>,
    >(
        &'lua mut self,
        sig: &CallbackSignature<'lua, A, R>,
        args: A,
    ) -> anyhow::Result<R> {
        let name = sig.name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();
        async_call_callback_non_default(&lua.lua, sig, args).await
    }

    pub async fn async_call_callback_non_default_opt<
        'lua,
        A: IntoLuaMulti<'lua> + Clone,
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
                .named_registry_value::<mlua::Value>(&decorated_name)?
            {
                Value::Table(tbl) => {
                    for func in tbl.sequence_values::<mlua::Function>() {
                        let func = func?;
                        let _timer = latency_timer(name);
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
                let _timer = latency_timer(name);
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
    pub async fn async_call_ctor<'lua, A: IntoLuaMulti<'lua> + Clone>(
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
            .named_registry_value::<mlua::Function>(&decorated_name)?;

        let _timer = latency_timer(name);
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

pub async fn async_call_callback<
    'lua,
    A: IntoLuaMulti<'lua> + Clone,
    R: FromLuaMulti<'lua> + Default,
>(
    lua: &'lua Lua,
    sig: &CallbackSignature<'lua, A, R>,
    args: A,
) -> anyhow::Result<R> {
    let name = sig.name();
    let decorated_name = sig.decorated_name();

    if sig.allow_multiple() {
        return match lua.named_registry_value::<mlua::Value>(&decorated_name)? {
            Value::Table(tbl) => {
                for func in tbl.sequence_values::<mlua::Function>() {
                    let func = func?;
                    let _timer = latency_timer(name);
                    let result: mlua::MultiValue = func.call_async(args.clone()).await?;
                    if result.is_empty() {
                        // Continue with other handlers
                        continue;
                    }
                    let result = R::from_lua_multi(result, lua)?;
                    return Ok(result);
                }
                Ok(R::default())
            }
            _ => Ok(R::default()),
        };
    }

    match lua.named_registry_value::<mlua::Function>(&decorated_name) {
        Ok(func) => {
            let _timer = latency_timer(name);
            Ok(func.call_async(args.clone()).await?)
        }
        _ => Ok(R::default()),
    }
}

pub async fn async_call_callback_non_default<
    'lua,
    A: IntoLuaMulti<'lua> + Clone,
    R: FromLuaMulti<'lua>,
>(
    lua: &'lua Lua,
    sig: &CallbackSignature<'lua, A, R>,
    args: A,
) -> anyhow::Result<R> {
    let name = sig.name();
    let decorated_name = sig.decorated_name();

    if sig.allow_multiple() {
        match lua.named_registry_value::<mlua::Value>(&decorated_name)? {
            Value::Table(tbl) => {
                for func in tbl.sequence_values::<mlua::Function>() {
                    let func = func?;
                    let _timer = latency_timer(name);
                    let result: mlua::MultiValue = func.call_async(args.clone()).await?;
                    if result.is_empty() {
                        // Continue with other handlers
                        continue;
                    }
                    let result = R::from_lua_multi(result, lua)?;
                    return Ok(result);
                }
            }
            _ => {}
        };
        anyhow::bail!("invalid return type for {name} event");
    }

    match lua.named_registry_value::<mlua::Function>(&decorated_name) {
        Ok(func) => {
            let _timer = latency_timer(name);
            Ok(func.call_async(args.clone()).await?)
        }
        _ => anyhow::bail!("Event {name} has not been registered"),
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

/// Given a name path like `foo` or `foo.bar.baz`, sets up the module
/// registry hierarchy to instantiate that path.
/// Returns the leaf node of that path to allow the caller to
/// register/assign functions etc. into it
pub fn get_or_create_sub_module<'lua>(
    lua: &'lua Lua,
    name_path: &str,
) -> anyhow::Result<mlua::Table<'lua>> {
    let mut parent = get_or_create_module(lua, "kumo")?;
    let mut path_so_far = String::new();

    for name in name_path.split('.') {
        if !path_so_far.is_empty() {
            path_so_far.push('.');
        }
        path_so_far.push_str(name);

        let sub = parent.get(name)?;
        match sub {
            Value::Nil => {
                let sub = lua.create_table()?;
                parent.set(name, sub.clone())?;
                parent = sub;
            }
            Value::Table(sub) => {
                parent = sub;
            }
            wat => anyhow::bail!(
                "cannot register module kumo.{path_so_far} as it is already set to a value of type {}",
                wat.type_name()
            ),
        }
    }

    Ok(parent)
}

/// Helper for mapping back to lua errors
pub fn any_err<E: std::fmt::Display>(err: E) -> mlua::Error {
    mlua::Error::external(format!("{err:#}"))
}

/// Convert from a lua value to a deserializable type,
/// with a slightly more helpful error message in case of failure.
pub fn from_lua_value<'lua, R>(lua: &'lua Lua, value: mlua::Value<'lua>) -> mlua::Result<R>
where
    R: serde::de::DeserializeOwned,
{
    let value_cloned = value.clone();
    lua.from_value(value).map_err(|err| {
        let mut serializer = serde_json::Serializer::new(Vec::new());
        let serialized = match value_cloned.serialize(&mut serializer) {
            Ok(_) => String::from_utf8_lossy(&serializer.into_inner()).to_string(),
            Err(err) => format!("<unable to encode as json: {err:#}>"),
        };
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
    A: IntoLuaMulti<'lua>,
    R: FromLuaMulti<'lua>,
{
    marker: std::marker::PhantomData<&'lua (A, R)>,
    allow_multiple: bool,
    name: Cow<'static, str>,
}

impl<'lua, A, R> CallbackSignature<'lua, A, R>
where
    A: IntoLuaMulti<'lua>,
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

    /// Make sure that you call .register() on this from
    /// eg: mod_kumo::register in order for it to be instantiated
    /// and visible to the config loader
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
    CALLBACK_ALLOWS_MULTIPLE.lock().contains(name)
}

pub fn decorate_callback_name(name: &str) -> String {
    format!("kumomta-on-{name}")
}

pub fn serialize_options() -> mlua::SerializeOptions {
    mlua::SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false)
}
