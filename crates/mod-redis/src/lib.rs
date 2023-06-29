use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_module};
use mlua::{Lua, MultiValue, UserData, UserDataMethods, Value};
use once_cell::sync::Lazy;
use r2d2::{ManageConnection, Pool, PooledConnection};
use redis::cluster::{ClusterClient, ClusterConnection};
pub use redis::{
    cmd, Cmd, FromRedisValue, RedisError, Script, ScriptInvocation, Value as RedisValue,
};
use redis::{Client, Connection, ConnectionLike, RedisWrite, ToRedisArgs};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub mod test;

static POOLS: Lazy<Mutex<HashMap<RedisConnKey, Pool<ClientWrapper>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
pub struct RedisConnection(Arc<RedisConnKey>);

impl RedisConnection {
    pub fn query_blocking(&self, cmd: &Cmd) -> anyhow::Result<RedisValue> {
        let mut conn = self.0.connect_blocking()?;
        Ok(conn.req_command(cmd)?)
    }

    pub fn invoke_blocking(&self, script: ScriptInvocation<'_>) -> anyhow::Result<RedisValue> {
        let mut conn = self.0.connect_blocking()?;
        Ok(script.invoke(&mut conn)?)
    }

    pub async fn query(&self, cmd: Cmd) -> anyhow::Result<RedisValue> {
        let me = self.clone();
        tokio::task::Builder::new()
            .name("redis query")
            .spawn_blocking(move || me.query_blocking(&cmd))?
            .await?
    }

    pub async fn invoke_script(
        &self,
        script: ScriptInvocation<'static>,
    ) -> anyhow::Result<RedisValue> {
        let me = self.clone();
        tokio::task::Builder::new()
            .name("redis script invocation")
            .spawn_blocking(move || me.invoke_blocking(script))?
            .await?
    }
}

fn redis_value_to_lua<'lua>(lua: &'lua Lua, value: RedisValue) -> mlua::Result<Value> {
    Ok(match value {
        RedisValue::Nil => Value::Nil,
        RedisValue::Int(i) => Value::Integer(i),
        RedisValue::Data(bytes) => Value::String(lua.create_string(&bytes)?),
        RedisValue::Bulk(values) => {
            let array = lua.create_table()?;
            for v in values {
                array.push(redis_value_to_lua(lua, v)?)?;
            }
            Value::Table(array)
        }
        RedisValue::Status(s) => Value::String(lua.create_string(&s)?),
        RedisValue::Okay => Value::Boolean(true),
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

    fn is_single_arg(&self) -> bool {
        match self.0 {
            JsonValue::Array(array) => array.len() == 1,
            JsonValue::Null => false,
            JsonValue::Object(map) => map.len() <= 1,
            JsonValue::Number(_) | JsonValue::Bool(_) | JsonValue::String(_) => true,
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
    /// Maximum number of connections managed by the pool.
    /// Default is 10
    #[serde(default)]
    pub pool_size: Option<u32>,
}

pub enum ClientWrapper {
    Single(Client),
    Cluster(ClusterClient),
}

pub enum ConnectionWrapper {
    Single(Connection),
    Cluster(ClusterConnection),
}

impl ConnectionLike for ConnectionWrapper {
    fn req_packed_command(&mut self, cmd: &[u8]) -> Result<RedisValue, RedisError> {
        match self {
            ConnectionWrapper::Single(c) => c.req_packed_command(cmd),
            ConnectionWrapper::Cluster(c) => c.req_packed_command(cmd),
        }
    }

    fn req_packed_commands(
        &mut self,
        cmd: &[u8],
        offset: usize,
        count: usize,
    ) -> Result<Vec<RedisValue>, RedisError> {
        match self {
            ConnectionWrapper::Single(c) => c.req_packed_commands(cmd, offset, count),
            ConnectionWrapper::Cluster(c) => c.req_packed_commands(cmd, offset, count),
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            ConnectionWrapper::Single(c) => c.get_db(),
            ConnectionWrapper::Cluster(c) => c.get_db(),
        }
    }

    fn check_connection(&mut self) -> bool {
        match self {
            ConnectionWrapper::Single(c) => c.check_connection(),
            ConnectionWrapper::Cluster(c) => c.check_connection(),
        }
    }

    fn is_open(&self) -> bool {
        match self {
            ConnectionWrapper::Single(c) => c.is_open(),
            ConnectionWrapper::Cluster(c) => c.is_open(),
        }
    }

    fn req_command(&mut self, cmd: &Cmd) -> Result<RedisValue, RedisError> {
        match self {
            ConnectionWrapper::Single(c) => c.req_command(cmd),
            ConnectionWrapper::Cluster(c) => c.req_command(cmd),
        }
    }
}

impl ManageConnection for ClientWrapper {
    type Connection = ConnectionWrapper;
    type Error = RedisError;

    fn connect(&self) -> Result<ConnectionWrapper, RedisError> {
        match self {
            ClientWrapper::Single(client) => Ok(ConnectionWrapper::Single(client.connect()?)),
            ClientWrapper::Cluster(client) => Ok(ConnectionWrapper::Cluster(client.connect()?)),
        }
    }

    fn is_valid(&self, conn: &mut ConnectionWrapper) -> Result<(), RedisError> {
        match (self, conn) {
            (ClientWrapper::Single(client), ConnectionWrapper::Single(conn)) => {
                client.is_valid(conn)
            }
            (ClientWrapper::Cluster(client), ConnectionWrapper::Cluster(conn)) => {
                client.is_valid(conn)
            }
            _ => unreachable!(),
        }
    }

    fn has_broken(&self, conn: &mut ConnectionWrapper) -> bool {
        match (self, conn) {
            (ClientWrapper::Single(client), ConnectionWrapper::Single(conn)) => {
                client.has_broken(conn)
            }
            (ClientWrapper::Cluster(client), ConnectionWrapper::Cluster(conn)) => {
                client.has_broken(conn)
            }
            _ => unreachable!(),
        }
    }
}

impl RedisConnKey {
    pub fn build_client(&self) -> anyhow::Result<ClientWrapper> {
        match &self.node {
            NodeSpec::Single(node) => Ok(ClientWrapper::Single(
                Client::open(node.to_string())
                    .with_context(|| format!("building redis client {self:?}"))?,
            )),
            NodeSpec::Cluster(nodes) => {
                let mut builder = ClusterClient::builder(nodes.clone());
                if self.read_from_replicas {
                    builder = builder.read_from_replicas();
                }
                if let Some(user) = &self.username {
                    builder = builder.username(user.to_string());
                }
                if let Some(pass) = &self.password {
                    builder = builder.password(pass.to_string());
                }

                Ok(ClientWrapper::Cluster(builder.build().with_context(
                    || format!("building redis client {self:?}"),
                )?))
            }
        }
    }

    pub fn get_pool(&self) -> anyhow::Result<Pool<ClientWrapper>> {
        let mut pools = POOLS.lock().unwrap();
        if let Some(p) = pools.get(self) {
            return Ok(p.clone());
        }

        let mut p = Pool::builder();
        if let Some(size) = self.pool_size {
            p = p.max_size(size);
        }

        let client = self.build_client()?;

        let p = p.build(client)?;
        pools.insert(self.clone(), p.clone());
        Ok(p)
    }

    pub fn connect_blocking(&self) -> anyhow::Result<PooledConnection<ClientWrapper>> {
        Ok(self.get_pool()?.get()?)
    }

    pub fn open_blocking(&self) -> anyhow::Result<RedisConnection> {
        self.get_pool()?;
        Ok(RedisConnection(Arc::new(self.clone())))
    }

    pub async fn open(&self) -> anyhow::Result<RedisConnection> {
        let me = self.clone();
        tokio::task::Builder::new()
            .name("open redis")
            .spawn_blocking(move || me.open_blocking())?
            .await?
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let redis_mod = get_or_create_module(lua, "redis")?;

    redis_mod.set(
        "open",
        lua.create_async_function(|lua, key: Value| async move {
            let key: RedisConnKey = from_lua_value(lua, key)?;
            key.open().await.map_err(any_err)
        })?,
    )?;

    Ok(())
}
