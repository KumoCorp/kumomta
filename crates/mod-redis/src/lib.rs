use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_module};
use deadpool::managed::{Manager, Metrics, Pool, RecycleError, RecycleResult};
use kumo_prometheus::declare_metric;
use mlua::{Lua, MultiValue, UserData, UserDataMethods, Value};
use redis::aio::{ConnectionLike, ConnectionManager, ConnectionManagerConfig};
use redis::cluster::ClusterClient;
use redis::cluster_async::ClusterConnection;
pub use redis::{
    cmd, Cmd, FromRedisValue, RedisError, Script, ScriptInvocation, Value as RedisValue,
};
use redis::{
    Client, ConnectionInfo, IntoConnectionInfo, Pipeline, RedisFuture, RedisWrite, ToRedisArgs,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub mod test;

static POOLS: LazyLock<Mutex<HashMap<RedisConnKey, Pool<ClientManager>>>> =
    LazyLock::new(Mutex::default);

pub struct ClientManager(ClientWrapper);

impl Manager for ClientManager {
    type Type = ConnectionWrapper;
    type Error = anyhow::Error;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        let c = self.0.connect().await?;
        Ok(c)
    }

    async fn recycle(
        &self,
        conn: &mut Self::Type,
        _metrics: &Metrics,
    ) -> RecycleResult<anyhow::Error> {
        conn.ping()
            .await
            .map_err(|err| RecycleError::message(format!("{err:#}")))
    }
}

declare_metric! {
/// The latency of an operation talking to Redis.
///
/// {{since('dev')}}
///
/// The `service` key represents the redis server/service. It is not
/// a direct match to a server name as it is really a hash of the
/// overall redis configuration information used in the client.
/// It might look something like:
/// `redis://127.0.0.1:24419,redis://127.0.0.1:7779,redis://127.0.0.1:29469-2ce79dd1`
/// for a cluster configuration, or `redis://127.0.0.1:16267-f4da6e64`
/// for a single node cluster configuration.
/// You should anticipate that the `-HEX` suffix can and will change
/// in an unspecified way as you vary the redis connection parameters.
///
/// The `operation` key indicates the operation, which can be a `ping`,
/// a `query` or a `script`.
///
/// `status` will be either `ok` or `error` to indicate whether this
/// is tracking a successful or failed operation.
///
/// Since histograms track a count of operations, you can track the
/// rate of `redis_operation_latency_count` where `status=error`
/// to have an indication of the failure rate of redis operations.
static REDIS_LATENCY:  HistogramVec("redis_operation_latency",
    &["service", "operation", "status"]);
}

#[derive(Debug)]
struct KeyAndLabel {
    key: RedisConnKey,
    label: String,
}

#[derive(Clone, Debug)]
pub struct RedisConnection(Arc<KeyAndLabel>);

impl RedisConnection {
    async fn sample_latency<T, E>(
        &self,
        operation: &str,
        fut: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let now = Instant::now();
        let result = (fut).await;
        let elapsed = now.elapsed().as_secs_f64();
        let status = if result.is_ok() { "ok" } else { "error" };

        if let Ok(hist) =
            REDIS_LATENCY.get_metric_with_label_values(&[self.0.label.as_str(), operation, status])
        {
            hist.observe(elapsed);
        }

        result
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        self.sample_latency("ping", async {
            let pool = self.0.key.get_pool()?;
            let mut conn = pool.get().await.map_err(|err| anyhow::anyhow!("{err:#}"))?;
            conn.ping().await
        })
        .await
    }

    pub async fn query(&self, cmd: Cmd) -> anyhow::Result<RedisValue> {
        self.sample_latency("query", async {
            let pool = self.0.key.get_pool()?;
            let mut conn = pool.get().await.map_err(|err| anyhow::anyhow!("{err:#}"))?;
            Ok(cmd.query_async(&mut *conn).await?)
        })
        .await
    }

    pub async fn invoke_script(
        &self,
        script: ScriptInvocation<'static>,
    ) -> anyhow::Result<RedisValue> {
        self.sample_latency("script", async {
            let pool = self.0.key.get_pool()?;
            let mut conn = pool.get().await.map_err(|err| anyhow::anyhow!("{err:#}"))?;
            Ok(script.invoke_async(&mut *conn).await?)
        })
        .await
    }
}

fn redis_value_to_lua(lua: &Lua, value: RedisValue) -> mlua::Result<Value> {
    Ok(match value {
        RedisValue::Nil => Value::Nil,
        RedisValue::Int(i) => Value::Integer(i),
        RedisValue::Boolean(i) => Value::Boolean(i),
        RedisValue::BigNumber(i) => Value::String(lua.create_string(i.to_string())?),
        RedisValue::Double(i) => Value::Number(i),
        RedisValue::BulkString(bytes) => Value::String(lua.create_string(&bytes)?),
        RedisValue::SimpleString(s) => Value::String(lua.create_string(&s)?),
        RedisValue::Map(pairs) => {
            let map = lua.create_table()?;
            for (k, v) in pairs {
                let k = redis_value_to_lua(lua, k)?;
                let v = redis_value_to_lua(lua, v)?;
                map.set(k, v)?;
            }
            Value::Table(map)
        }
        RedisValue::Array(values) => {
            let array = lua.create_table()?;
            for v in values {
                array.push(redis_value_to_lua(lua, v)?)?;
            }
            Value::Table(array)
        }
        RedisValue::Set(values) => {
            let array = lua.create_table()?;
            for v in values {
                array.push(redis_value_to_lua(lua, v)?)?;
            }
            Value::Table(array)
        }
        RedisValue::Attribute { data, attributes } => {
            let map = lua.create_table()?;
            for (k, v) in attributes {
                let k = redis_value_to_lua(lua, k)?;
                let v = redis_value_to_lua(lua, v)?;
                map.set(k, v)?;
            }

            let attribute = lua.create_table()?;
            attribute.set("data", redis_value_to_lua(lua, *data)?)?;
            attribute.set("attributes", map)?;

            Value::Table(attribute)
        }
        RedisValue::VerbatimString { format, text } => {
            let vstr = lua.create_table()?;
            vstr.set("format", format.to_string())?;
            vstr.set("text", text)?;
            Value::Table(vstr)
        }
        RedisValue::ServerError(_) => {
            return Err(value
                .extract_error()
                .map_err(mlua::Error::external)
                .unwrap_err());
        }
        RedisValue::Okay => Value::Boolean(true),
        RedisValue::Push { kind, data } => {
            let array = lua.create_table()?;
            for v in data {
                let v = redis_value_to_lua(lua, v)?;
                array.push(v)?;
            }

            let push = lua.create_table()?;
            push.set("data", array)?;
            push.set("kind", kind.to_string())?;

            Value::Table(push)
        }
    })
}

impl UserData for RedisConnection {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("query", |lua, this, params: MultiValue| async move {
            let mut args = vec![];
            for p in params {
                args.push(from_lua_value(&lua, p)?);
            }
            let cmd = build_cmd(args).map_err(any_err)?;
            let result = this.query(cmd).await.map_err(any_err)?;
            redis_value_to_lua(&lua, result)
        });
    }
}

struct RedisJsonValue<'a>(&'a JsonValue);

impl ToRedisArgs for RedisJsonValue<'_> {
    fn write_redis_args<W>(&self, write: &mut W)
    where
        W: ?Sized + RedisWrite,
    {
        match self.0 {
            JsonValue::Null => {}
            JsonValue::Bool(b) => {
                b.write_redis_args(write);
            }
            JsonValue::Number(n) => n.to_string().write_redis_args(write),
            JsonValue::String(s) => s.write_redis_args(write),
            JsonValue::Array(array) => {
                for item in array {
                    RedisJsonValue(item).write_redis_args(write);
                }
            }
            JsonValue::Object(map) => {
                for (k, v) in map {
                    k.write_redis_args(write);
                    RedisJsonValue(v).write_redis_args(write);
                }
            }
        }
    }

    fn num_of_args(&self) -> usize {
        match self.0 {
            JsonValue::Array(array) => array.len(),
            JsonValue::Null => 1,
            JsonValue::Object(map) => map.len(),
            JsonValue::Number(_) | JsonValue::Bool(_) | JsonValue::String(_) => 1,
        }
    }
}

pub fn build_cmd(args: Vec<JsonValue>) -> anyhow::Result<Cmd> {
    let mut cmd = Cmd::new();
    for a in args {
        cmd.arg(RedisJsonValue(&a));
    }
    Ok(cmd)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(untagged)]
pub enum NodeSpec {
    /// A single, non-clustered redis node
    Single(String),
    /// List of redis URLs for hosts in the cluster
    Cluster(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct RedisConnKey {
    pub node: NodeSpec,
    /// Enables reading from replicas for all new connections
    #[serde(default)]
    pub read_from_replicas: bool,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub cluster: Option<bool>,
    /// Maximum number of connections managed by the pool.
    /// Default is 10
    #[serde(default)]
    pub pool_size: Option<usize>,
    #[serde(default, with = "duration_serde")]
    pub connect_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    pub recycle_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    pub wait_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    pub response_timeout: Option<Duration>,
}

pub enum ClientWrapper {
    Single(Client, ConnectionManagerConfig),
    Cluster(ClusterClient),
}

impl ClientWrapper {
    pub async fn connect(&self) -> anyhow::Result<ConnectionWrapper> {
        match self {
            Self::Single(client, config) => Ok(ConnectionWrapper::Single(
                ConnectionManager::new_with_config(client.clone(), config.clone()).await?,
            )),
            Self::Cluster(c) => Ok(ConnectionWrapper::Cluster(c.get_async_connection().await?)),
        }
    }
}

pub enum ConnectionWrapper {
    Single(ConnectionManager),
    Cluster(ClusterConnection),
}

impl ConnectionWrapper {
    pub async fn ping(&mut self) -> anyhow::Result<()> {
        Ok(redis::cmd("PING").query_async(self).await?)
    }
}

impl ConnectionLike for ConnectionWrapper {
    // Required methods
    fn req_packed_command<'a>(&'a mut self, cmd: &'a Cmd) -> RedisFuture<'a, RedisValue> {
        match self {
            Self::Single(c) => c.req_packed_command(cmd),
            Self::Cluster(c) => c.req_packed_command(cmd),
        }
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        cmd: &'a crate::Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisFuture<'a, Vec<RedisValue>> {
        match self {
            Self::Single(c) => c.req_packed_commands(cmd, offset, count),
            Self::Cluster(c) => c.req_packed_commands(cmd, offset, count),
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            Self::Single(c) => c.get_db(),
            Self::Cluster(c) => c.get_db(),
        }
    }
}

impl RedisConnKey {
    pub fn build_client(&self) -> anyhow::Result<ClientWrapper> {
        let cluster = self
            .cluster
            .unwrap_or(matches!(&self.node, NodeSpec::Cluster(_)));
        let nodes = match &self.node {
            NodeSpec::Single(node) => vec![node.to_string()],
            NodeSpec::Cluster(nodes) => nodes.clone(),
        };

        if cluster {
            let mut builder = ClusterClient::builder(nodes);
            if self.read_from_replicas {
                builder = builder.read_from_replicas();
            }
            if let Some(user) = &self.username {
                builder = builder.username(user.to_string());
            }
            if let Some(pass) = &self.password {
                builder = builder.password(pass.to_string());
            }
            if let Some(duration) = self.connect_timeout {
                builder = builder.connection_timeout(duration);
            }
            if let Some(duration) = self.response_timeout {
                builder = builder.response_timeout(duration);
            }

            Ok(ClientWrapper::Cluster(builder.build().with_context(
                || format!("building redis client {self:?}"),
            )?))
        } else {
            let mut config = ConnectionManagerConfig::new();
            if let Some(duration) = self.connect_timeout {
                config = config.set_connection_timeout(duration);
            }
            if let Some(duration) = self.response_timeout {
                config = config.set_response_timeout(duration);
            }

            let mut info: ConnectionInfo = nodes[0]
                .as_str()
                .into_connection_info()
                .with_context(|| format!("building redis client {self:?}"))?;
            if let Some(user) = &self.username {
                info.redis.username.replace(user.to_string());
            }
            if let Some(pass) = &self.password {
                info.redis.password.replace(pass.to_string());
            }

            Ok(ClientWrapper::Single(
                Client::open(info).with_context(|| format!("building redis client {self:?}"))?,
                config,
            ))
        }
    }

    pub fn get_pool(&self) -> anyhow::Result<Pool<ClientManager>> {
        let mut pools = POOLS.lock().unwrap();
        if let Some(pool) = pools.get(self) {
            return Ok(pool.clone());
        }

        let client = self.build_client()?;
        let mut builder = Pool::builder(ClientManager(client))
            .runtime(deadpool::Runtime::Tokio1)
            .create_timeout(self.connect_timeout)
            .recycle_timeout(self.recycle_timeout)
            .wait_timeout(self.wait_timeout);

        if let Some(limit) = self.pool_size {
            builder = builder.max_size(limit);
        }

        let pool = builder.build()?;

        pools.insert(self.clone(), pool.clone());

        Ok(pool)
    }

    pub fn open(&self) -> anyhow::Result<RedisConnection> {
        self.build_client()?;
        Ok(RedisConnection(Arc::new(KeyAndLabel {
            key: self.clone(),
            label: self.hash_label(),
        })))
    }

    /// Produces a human readable label string that is representitive
    /// of this RedisConnKey.  We pull out the node and username to
    /// include in the label.
    /// Now, since the entire RedisConnKey is the actual key, that
    /// readable subset is not sufficient to uniquely identify the
    /// entry in the pool, although in reality it is probably OK,
    /// there exists the possibility that eg: on a config update,
    /// multiple entries have the same list of nodes but different
    /// auth or other parameters.
    /// To smooth over such a transition, we'll include a basic
    /// crc32 of the entire RedisConnKey in the label that is
    /// returned.  This will probably be sufficient to avoid
    /// an obvious collision between such names, but it will not
    /// guarantee it.
    /// This is a best effort really; I doubt that this will cause
    /// any meaningful issues in practice, as the intended use case
    /// for this label string is to sample metrics rather than to
    /// guarantee isolation.
    pub fn hash_label(&self) -> String {
        use crc32fast::Hasher;
        use std::hash::Hash;
        let mut hasher = Hasher::new();
        self.hash(&mut hasher);
        let crc = hasher.finalize();

        let mut label = String::new();
        if let Some(user) = &self.username {
            label.push_str(user);
            label.push('@');
        }
        match &self.node {
            NodeSpec::Single(node) => {
                label.push_str(node);
            }
            NodeSpec::Cluster(nodes) => {
                for (idx, node) in nodes.iter().enumerate() {
                    if idx > 0 {
                        label.push(',');
                    }
                    label.push_str(node);
                }
            }
        }
        label.push_str(&format!("-{crc:08x}"));
        label
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let redis_mod = get_or_create_module(lua, "redis")?;

    redis_mod.set(
        "open",
        lua.create_function(move |lua, key: Value| {
            let key: RedisConnKey = from_lua_value(lua, key)?;
            key.open().map_err(any_err)
        })?,
    )?;

    Ok(())
}
