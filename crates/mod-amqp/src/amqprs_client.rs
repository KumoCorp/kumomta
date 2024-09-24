use amqprs::callbacks::{DefaultChannelCallback, DefaultConnectionCallback};
use amqprs::channel::{BasicPublishArguments, Channel, ConfirmSelectArguments};
use amqprs::connection::{Connection, OpenConnectionArguments};
use amqprs::tls::TlsAdaptor;
use amqprs::{BasicProperties, FieldTable, TimeStamp};
use deadpool::managed::{Manager, Metrics, Pool, RecycleError, RecycleResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static POOLS: LazyLock<Mutex<HashMap<ConnectionInfo, Pool<ConnectionManager>>>> =
    LazyLock::new(Mutex::default);

#[derive(Clone, Debug, Serialize, Deserialize, Hash, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInfo {
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub vhost: Option<String>,
    pub connection_name: Option<String>,
    pub heartbeat: Option<u16>,
    #[serde(default)]
    pub enable_tls: bool,
    pub root_ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_private_key: Option<String>,

    // TODO: not fully implemented
    #[serde(default)]
    pub confirm_select: bool,

    #[serde(default)]
    pub pool_size: Option<usize>,
    #[serde(default, with = "duration_serde")]
    pub connect_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    pub recycle_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    pub wait_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    pub publish_timeout: Option<Duration>,
}

pub struct ConnectionManager(ConnectionInfo);
pub struct ConnectionWithInfo {
    connection: Connection,
    // TODO: when wiring up confirm_select, we need to look
    // at this to see whether we should await for the publish
    #[allow(unused)]
    info: ConnectionInfo,
}

pub struct ConnectionAndChannel {
    connection: ConnectionWithInfo,
    channel: Channel,
}

impl Manager for ConnectionManager {
    type Type = ConnectionAndChannel;
    type Error = anyhow::Error;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        let connection = self.0.connect().await?;

        connection
            .register_callback(DefaultConnectionCallback)
            .await?;

        let channel = connection.open_channel(None).await?;
        channel.register_callback(DefaultChannelCallback).await?;
        if self.0.confirm_select {
            channel
                .confirm_select(ConfirmSelectArguments::default())
                .await?;
        }

        Ok(ConnectionAndChannel {
            connection: ConnectionWithInfo {
                connection,
                info: self.0.clone(),
            },
            channel,
        })
    }

    async fn recycle(
        &self,
        conn: &mut Self::Type,
        _metrics: &Metrics,
    ) -> RecycleResult<anyhow::Error> {
        if conn.connection.connection.is_open() && conn.channel.is_open() {
            Ok(())
        } else {
            Err(RecycleError::message("channel/connection is closed"))
        }
    }
}

impl ConnectionInfo {
    pub async fn connect(&self) -> anyhow::Result<Connection> {
        let mut args = OpenConnectionArguments::new(
            &self.host,
            self.port.unwrap_or(5672),
            self.username.as_deref().unwrap_or("guest"),
            self.password.as_deref().unwrap_or("guest"),
        );
        if let Some(vhost) = &self.vhost {
            args.virtual_host(vhost);
        }
        if let Some(name) = &self.connection_name {
            args.connection_name(name);
        }
        if let Some(hb) = self.heartbeat {
            args.heartbeat(hb);
        }
        if self.enable_tls {
            let adaptor = match (&self.client_cert, &self.client_private_key) {
                (Some(cert), Some(key)) => TlsAdaptor::with_client_auth(
                    self.root_ca_cert.as_deref().map(Path::new),
                    Path::new(cert),
                    Path::new(key),
                    self.host.to_string(),
                )?,
                (None, None) => TlsAdaptor::without_client_auth(
                    self.root_ca_cert.as_deref().map(Path::new),
                    self.host.to_string(),
                )?,
                _ => anyhow::bail!(
                    "Either both client_cert and client_private_key must be specified, or neither"
                ),
            };
            args.tls_adaptor(adaptor);
        }

        let connection = Connection::open(&args).await?;

        Ok(connection)
    }

    pub fn get_pool(&self) -> anyhow::Result<Pool<ConnectionManager>> {
        let mut pools = POOLS.lock().unwrap();
        if let Some(pool) = pools.get(self) {
            return Ok(pool.clone());
        }

        let mut builder = Pool::builder(ConnectionManager(self.clone()))
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
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct PublishParams {
    pub routing_key: String,
    pub payload: String,
    pub connection: ConnectionInfo,

    pub app_id: Option<String>,
    pub cluster_id: Option<String>,
    pub content_encoding: Option<String>,
    pub content_type: Option<String>,
    pub correlation_id: Option<String>,
    pub delivery_mode: Option<u8>,
    pub expiration: Option<String>,
    pub headers: Option<FieldTable>,
    pub message_id: Option<String>,
    pub message_type: Option<String>,
    pub priority: Option<u8>,
    pub reply_to: Option<String>,
    pub timestamp: Option<TimeStamp>,
    pub user_id: Option<String>,

    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub mandatory: bool,
    #[serde(default)]
    pub immediate: bool,
}

pub async fn publish(params: PublishParams) -> anyhow::Result<()> {
    kumo_server_runtime::get_main_runtime()
        .spawn(async move {
            let connection = params
                .connection
                .get_pool()?
                .get()
                .await
                .map_err(|err| anyhow::anyhow!("{err:#}"))?;

            let mut props = BasicProperties::default();
            if let Some(v) = &params.app_id {
                props.with_app_id(v);
            }
            if let Some(v) = &params.cluster_id {
                props.with_cluster_id(v);
            }
            if let Some(v) = &params.content_encoding {
                props.with_content_encoding(v);
            }
            if let Some(v) = &params.content_type {
                props.with_content_type(v);
            }
            if let Some(v) = &params.correlation_id {
                props.with_correlation_id(v);
            }
            if let Some(v) = params.delivery_mode {
                props.with_delivery_mode(v);
            }
            if let Some(v) = &params.expiration {
                props.with_expiration(v);
            }
            if let Some(v) = params.headers {
                props.with_headers(v);
            }
            if let Some(v) = &params.message_id {
                props.with_message_id(v);
            }
            if let Some(v) = &params.message_type {
                props.with_message_type(v);
            }
            if let Some(v) = params.priority {
                props.with_priority(v);
            }
            if let Some(v) = &params.reply_to {
                props.with_reply_to(v);
            }
            if let Some(v) = params.timestamp {
                props.with_timestamp(v);
            }
            if let Some(v) = &params.user_id {
                props.with_user_id(v);
            }

            let args = BasicPublishArguments {
                exchange: params.exchange,
                routing_key: params.routing_key,
                mandatory: params.mandatory,
                immediate: params.immediate,
            };

            let timeout_duration = params
                .connection
                .publish_timeout
                .unwrap_or_else(|| Duration::from_secs(60));

            tokio::time::timeout(
                timeout_duration,
                connection
                    .channel
                    .basic_publish(props, params.payload.into_bytes(), args),
            )
            .await??;

            Ok(())
        })
        .await?
}
