use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_module};
use deadpool::managed::{Manager, Metrics, Pool, RecycleError, RecycleResult};
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
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

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

#[derive(Clone, Debug)]
pub struct RedisConnection(Arc<RedisConnKey>);

impl RedisConnection {
    pub async fn ping(&self) -> anyhow::Result<()> {
        let pool = self.0.get_pool()?;
        let mut conn = pool.get().await.map_err(|err| anyhow::anyhow!("{err:#}"))?;
        conn.ping().await
    }

    pub async fn query(&self, cmd: Cmd) -> anyhow::Result<RedisValue> {
        let pool = self.0.get_pool()?;
        let mut conn = pool.get().await.map_err(|err| anyhow::anyhow!("{err:#}"))?;
        Ok(cmd.query_async(&mut *conn).await?)
    }

    pub async fn invoke_script(
        &self,
        script: ScriptInvocation<'static>,
    ) -> anyhow::Result<RedisValue> {
        let pool = self.0.get_pool()?;
        let mut conn = pool.get().await.map_err(|err| anyhow::anyhow!("{err:#}"))?;
        Ok(script.invoke_async(&mut *conn).await?)
    }
}

fn redis_value_to_lua<'lua>(lua: &'lua Lua, value: RedisValue) -> mlua::Result<Value> {
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
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("query", |lua, this, params: MultiValue| async move {
            let mut args = vec![];
            for p in params {
                args.push(from_lua_value(lua, p)?);
            }
            let cmd = build_cmd(args).map_err(any_err)?;
            let result = this.query(cmd).await.map_err(any_err)?;
            redis_value_to_lua(lua, result)
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
        Ok(RedisConnection(Arc::new(self.clone())))
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
