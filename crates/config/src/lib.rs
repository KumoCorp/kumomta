use crate::epoch::{get_current_epoch, ConfigEpoch};
use crate::pool::{pool_get, pool_put};
pub use crate::pool::{set_gc_on_put, set_max_age, set_max_spare, set_max_use};
use anyhow::Context;
use mlua::{
    FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Lua, LuaSerdeExt, MetaMethod, RegistryKey, Table,
    UserData, UserDataMethods, Value,
};
use parking_lot::FairMutex as Mutex;
pub use pastey as paste;
use prometheus::{CounterVec, HistogramTimer, HistogramVec};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Once};
use std::time::Instant;

pub mod epoch;
mod pool;

static POLICY_FILE: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| Mutex::new(None));
static FUNCS: LazyLock<Mutex<Vec<RegisterFunc>>> = LazyLock::new(|| Mutex::new(vec![]));
static LUA_LOAD_COUNT: LazyLock<metrics::Counter> = LazyLock::new(|| {
    metrics::describe_counter!(
        "lua_load_count",
        "how many times the policy lua script has been \
         loaded into a new context"
    );
    metrics::counter!("lua_load_count")
});
static LUA_COUNT: LazyLock<metrics::Gauge> = LazyLock::new(|| {
    metrics::describe_gauge!("lua_count", "the number of lua contexts currently alive");
    metrics::gauge!("lua_count")
});
static CALLBACK_ALLOWS_MULTIPLE: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub static VALIDATE_ONLY: AtomicBool = AtomicBool::new(false);
pub static VALIDATION_FAILED: AtomicBool = AtomicBool::new(false);
static LATENCY_HIST: LazyLock<HistogramVec> = LazyLock::new(|| {
    prometheus::register_histogram_vec!(
        "lua_event_latency",
        "how long a given lua event callback took",
        &["event"]
    )
    .unwrap()
});
static EVENT_STARTED_COUNT: LazyLock<CounterVec> = LazyLock::new(|| {
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
    epoch: ConfigEpoch,
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

pub async fn set_policy_path(path: PathBuf) -> anyhow::Result<()> {
    POLICY_FILE.lock().replace(path);
    let config = load_config().await?;
    config.put();
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
    let epoch = get_current_epoch();

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

    register_declared_events();

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
        func.call_async::<()>(()).await?;
    }
    LUA_COUNT.increment(1.);

    Ok(LuaConfig {
        inner: Some(LuaConfigInner {
            lua,
            created,
            use_count: 1,
            epoch,
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

    /// Convert an array of args into a MultiValue that can be passed
    /// to a callback signature
    pub fn convert_args_to_multi<A: Serialize>(
        &self,
        args: &[A],
    ) -> anyhow::Result<mlua::MultiValue> {
        let lua = self.inner.as_ref().unwrap();
        let mut arg_vec = vec![];
        for a in args.iter() {
            arg_vec.push(lua.lua.to_value(a)?);
        }
        Ok(mlua::MultiValue::from_vec(arg_vec))
    }

    /// Intended to be used together with kumo.spawn_task
    pub async fn convert_args_and_call_callback<A: Serialize>(
        &mut self,
        sig: &CallbackSignature<Value, ()>,
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

    /// Explicitly put the config object back into its containing pool.
    /// Ideally we'd do this automatically when the object is dropped,
    /// but lua's garbage collection makes this problematic:
    /// if a future whose graph contains an async lua call within
    /// this config object is cancelled (eg: simply stopped without
    /// calling it again), and the config object is not explicitly garbage
    /// collected, any futures and data owned by any dependencies of
    /// the cancelled future remain alive until the next gc run,
    /// which can cause things like async locks and semaphores to
    /// have a lifetime extended by the maximum age of the lua context.
    ///
    /// The combat this, consumers of LuaConfig should explicitly
    /// call `config.put()` after successfully using the config
    /// object.
    ///
    /// Or framing it another way: consumers must not call `config.put()`
    /// if a transitive dep might have been cancelled.
    pub fn put(mut self) {
        if let Some(inner) = self.inner.take() {
            pool_put(inner);
        }
    }

    pub async fn async_call_callback<A: IntoLuaMulti + Clone, R: FromLuaMulti + Default>(
        &mut self,
        sig: &CallbackSignature<A, R>,
        args: A,
    ) -> anyhow::Result<R> {
        let name = sig.name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();
        async_call_callback(&lua.lua, sig, args).await
    }

    pub async fn async_call_callback_non_default<A: IntoLuaMulti + Clone, R: FromLuaMulti>(
        &mut self,
        sig: &CallbackSignature<A, R>,
        args: A,
    ) -> anyhow::Result<R> {
        let name = sig.name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();
        async_call_callback_non_default(&lua.lua, sig, args).await
    }

    pub async fn async_call_callback_non_default_opt<A: IntoLuaMulti + Clone, R: FromLua>(
        &mut self,
        sig: &CallbackSignature<A, Option<R>>,
        args: A,
    ) -> anyhow::Result<Option<R>> {
        let name = sig.name();
        let decorated_name = sig.decorated_name();
        self.set_current_event(name)?;
        let lua = self.inner.as_mut().unwrap();

        match lua
            .lua
            .named_registry_value::<mlua::Value>(&decorated_name)?
        {
            Value::Table(tbl) => {
                for func in tbl.sequence_values::<mlua::Function>().collect::<Vec<_>>() {
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
            Value::Function(func) => {
                sig.raise_error_if_allow_multiple()?;
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
            _ => Ok(None),
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
    pub async fn async_call_ctor<A: IntoLuaMulti + Clone>(
        &mut self,
        sig: &CallbackSignature<A, Value>,
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
    pub async fn with_registry_value<F, R, FUT>(
        &mut self,
        value: &RegistryKey,
        func: F,
    ) -> anyhow::Result<R>
    where
        R: FromLuaMulti,
        F: FnOnce(Value) -> anyhow::Result<FUT>,
        FUT: std::future::Future<Output = anyhow::Result<R>>,
    {
        let inner = self.inner.as_mut().unwrap();
        let value = inner.lua.registry_value(value)?;
        let future = (func)(value)?;
        future.await
    }
}

pub async fn async_call_callback<A: IntoLuaMulti + Clone, R: FromLuaMulti + Default>(
    lua: &Lua,
    sig: &CallbackSignature<A, R>,
    args: A,
) -> anyhow::Result<R> {
    let name = sig.name();
    let decorated_name = sig.decorated_name();

    match lua.named_registry_value::<mlua::Value>(&decorated_name)? {
        Value::Table(tbl) => {
            for func in tbl.sequence_values::<mlua::Function>().collect::<Vec<_>>() {
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
        Value::Function(func) => {
            sig.raise_error_if_allow_multiple()?;
            let _timer = latency_timer(name);
            Ok(func.call_async(args.clone()).await?)
        }
        _ => Ok(R::default()),
    }
}

pub async fn async_call_callback_non_default<A: IntoLuaMulti + Clone, R: FromLuaMulti>(
    lua: &Lua,
    sig: &CallbackSignature<A, R>,
    args: A,
) -> anyhow::Result<R> {
    let name = sig.name();
    let decorated_name = sig.decorated_name();

    match lua.named_registry_value::<mlua::Value>(&decorated_name)? {
        Value::Table(tbl) => {
            for func in tbl.sequence_values::<mlua::Function>().collect::<Vec<_>>() {
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
            anyhow::bail!("invalid return type for {name} event");
        }
        Value::Function(func) => {
            sig.raise_error_if_allow_multiple()?;
            let _timer = latency_timer(name);
            Ok(func.call_async(args.clone()).await?)
        }
        _ => anyhow::bail!("Event {name} has not been registered"),
    }
}

pub fn get_or_create_module(lua: &Lua, name: &str) -> anyhow::Result<mlua::Table> {
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
pub fn get_or_create_sub_module(lua: &Lua, name_path: &str) -> anyhow::Result<mlua::Table> {
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

/// Provides implementations of __pairs, __index and __len metamethods
/// for a type that is Serialize and UserData.
/// Neither implementation is considered to be ideal, as we must
/// first serialize the value into a json Value which is then either
/// iterated over, or indexed to produce the appropriate result for
/// the metamethod.
pub fn impl_pairs_and_index<T, M>(methods: &mut M)
where
    T: UserData + Serialize,
    M: UserDataMethods<T>,
{
    methods.add_meta_method(MetaMethod::Pairs, move |lua, this, _: ()| {
        let Ok(serde_json::Value::Object(map)) = serde_json::to_value(this).map_err(any_err) else {
            return Err(mlua::Error::external("must serialize to Map"));
        };

        let mut value_iter = map.into_iter();

        let iter_func = lua.create_function_mut(
            move |lua, (_state, _control): (Value, Value)| match value_iter.next() {
                Some((key, value)) => {
                    let key = lua.to_value(&key)?;
                    let value = lua.to_value(&value)?;
                    Ok((key, value))
                }
                None => Ok((Value::Nil, Value::Nil)),
            },
        )?;

        Ok((Value::Function(iter_func), Value::Nil, Value::Nil))
    });

    methods.add_meta_method(MetaMethod::Index, move |lua, this, field: Value| {
        let value = lua.to_value(this)?;
        match value {
            Value::Table(t) => t.get(field),
            _ => Ok(Value::Nil),
        }
    });

    methods.add_meta_method(MetaMethod::Len, move |lua, this, _: ()| {
        let value = lua.to_value(this)?;
        match value {
            Value::Table(v) => v.len(),
            Value::String(v) => Ok(v.as_bytes().len() as i64),
            _ => Ok(0),
        }
    });
}

/// This function will try to obtain a native lua representation
/// of the provided value. It does this by attempting to iterate
/// the pairs of any userdata it finds as either the value itself
/// or the values of a table value by recursively applying
/// materialize_to_lua_value to the value.
/// This produces a lua value that can then be processed by the
/// Deserialize impl on Value.
pub fn materialize_to_lua_value(lua: &Lua, value: mlua::Value) -> mlua::Result<mlua::Value> {
    match value {
        mlua::Value::UserData(ud) => {
            let mt = ud.metatable()?;
            let Ok(pairs) = mt.get::<mlua::Function>("__pairs") else {
                let value = ud.into_lua(lua)?;
                return Err(mlua::Error::external(format!(
                    "cannot materialize_to_lua_value {value:?} \
                     because it has no __pairs metamethod"
                )));
            };
            let tbl = lua.create_table()?;
            let (iter_func, state, mut control): (mlua::Function, mlua::Value, mlua::Value) =
                pairs.call(mlua::Value::UserData(ud.clone()))?;

            loop {
                let (k, v): (mlua::Value, mlua::Value) =
                    iter_func.call((state.clone(), control))?;
                if k.is_nil() {
                    break;
                }

                tbl.set(k.clone(), materialize_to_lua_value(lua, v)?)?;
                control = k;
            }

            Ok(mlua::Value::Table(tbl))
        }
        mlua::Value::Table(t) => {
            let tbl = lua.create_table()?;
            for pair in t.pairs::<mlua::Value, mlua::Value>() {
                let (k, v) = pair?;
                tbl.set(k.clone(), materialize_to_lua_value(lua, v)?)?;
            }
            Ok(mlua::Value::Table(tbl))
        }
        value => Ok(value),
    }
}

/// Helper wrapper type for passing/returning serde encoded values from/to lua
pub struct SerdeWrappedValue<T>(pub T);

impl<T: serde::Serialize> SerdeWrappedValue<T> {
    pub fn to_lua_value(&self, lua: &Lua) -> mlua::Result<mlua::Value> {
        lua.to_value_with(&self.0, serialize_options())
    }
}

impl<T: serde::Serialize> IntoLua for SerdeWrappedValue<T> {
    fn into_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        lua.to_value_with(&self.0, serialize_options())
    }
}

impl<T: serde::de::DeserializeOwned> FromLua for SerdeWrappedValue<T> {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<SerdeWrappedValue<T>> {
        let inner: T = from_lua_value(lua, value)?;
        Ok(SerdeWrappedValue(inner))
    }
}

impl<T> std::ops::Deref for SerdeWrappedValue<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for SerdeWrappedValue<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Convert from a lua value to a deserializable type,
/// with a slightly more helpful error message in case of failure.
/// NOTE: the ", while processing" portion of the error messages generated
/// here is coupled with a regex in typing.lua!
pub fn from_lua_value<R>(lua: &Lua, value: mlua::Value) -> mlua::Result<R>
where
    R: serde::de::DeserializeOwned,
{
    let value_cloned = value.clone();
    match lua.from_value(value) {
        Ok(r) => Ok(r),
        Err(err) => match materialize_to_lua_value(lua, value_cloned.clone()) {
            Ok(materialized) => match lua.from_value(materialized.clone()) {
                Ok(r) => Ok(r),
                Err(err) => {
                    let mut serializer = serde_json::Serializer::new(Vec::new());
                    let serialized = match materialized.serialize(&mut serializer) {
                        Ok(_) => String::from_utf8_lossy(&serializer.into_inner()).to_string(),
                        Err(err) => format!("<unable to encode as json: {err:#}>"),
                    };
                    Err(mlua::Error::external(format!(
                        "{err:#}, while processing {serialized}"
                    )))
                }
            },
            Err(materialize_err) => Err(mlua::Error::external(format!(
                "{err:#}, while processing a userdata. \
                    Additionally, encountered {materialize_err:#} \
                    when trying to iterate the pairs of that userdata"
            ))),
        },
    }
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
pub struct CallbackSignature<A, R>
where
    A: IntoLuaMulti,
    R: FromLuaMulti,
{
    marker: std::marker::PhantomData<(A, R)>,
    allow_multiple: bool,
    name: Cow<'static, str>,
}

#[linkme::distributed_slice]
pub static CALLBACK_SIGNATURES: [fn()];

/// Helper for declaring a named event handler callback signature.
///
/// Usage looks like:
///
/// ```rust,ignore
/// declare_event! {
/// pub static GET_Q_CONFIG_SIG: Multiple(
///         "get_queue_config",
///         domain: &'static str,
///         tenant: Option<&'static str>,
///         campaign: Option<&'static str>,
///         routing_domain: Option<&'static str>,
///     ) -> QueueConfig;
/// }
/// ```
///
/// A handler can be either `Single` or `Multiple`, indicating whether
/// only a single registration or multiple registrations are permitted.
/// The string literal is the name of the event, followed by a fn-style
/// parameter list which names each parameter in sequence, followed by
/// the return value.  The names are not currently used in any way,
/// but enhance the readability of the code.
///
/// In addition to declaring the signature in a global, some glue
/// is generated that will register the signature appropriately
/// so that lua knows whether it is single or multiple and can
/// act appropriately when `kumo.on` is called.
#[macro_export]
macro_rules! declare_event {
    ($vis:vis static $sym:ident: Multiple($name:literal $(,)? $($param_name:ident: $args:ty),* $(,)? ) -> $ret:ty;) => {
        $vis static $sym: ::std::sync::LazyLock<
            $crate::CallbackSignature<($($args),*), $ret>> =
                ::std::sync::LazyLock::new(|| $crate::CallbackSignature::new_with_multiple($name));

        $crate::paste::paste! {
            #[linkme::distributed_slice($crate::CALLBACK_SIGNATURES)]
            static [<CALLBACK_SIG_REGISTER_ $sym>]: fn() = || {
                $sym.register();
            };
        }
    };
    ($vis:vis static $sym:ident: Single($name:literal $(,)? $($param_name:ident: $args:ty),* $(,)? ) -> $ret:ty;) => {
        $vis static $sym: ::std::sync::LazyLock<
            $crate::CallbackSignature<($($args),*), $ret>> =
                ::std::sync::LazyLock::new(|| $crate::CallbackSignature::new($name));

        $crate::paste::paste! {
            #[linkme::distributed_slice($crate::CALLBACK_SIGNATURES)]
            static [<CALLBACK_SIG_REGISTER_ $sym>]: fn() = || {
                $sym.register();
            };
        }
    };
}

/// For each event handler CallbackSignature that was declared via
/// `declare_event!`, call its `.register()` method to register
/// it so that `kumo.on` can give appropriate messaging if misused,
/// and so that runtime dispatch will work correctly.
///
/// This should be called once, prior to running any lua code.
fn register_declared_events() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for reg_func in CALLBACK_SIGNATURES {
            reg_func();
        }
    });
}

impl<A, R> CallbackSignature<A, R>
where
    A: IntoLuaMulti,
    R: FromLuaMulti,
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

    pub fn raise_error_if_allow_multiple(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.allow_multiple(),
            "handler {} is set to allow multiple handlers \
                    but is registered with a single instance. This indicates that \
                    register() was not called on the signature when initializing \
                    the lua context. Please report this issue to the KumoMTA team!",
            self.name
        );
        Ok(())
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
