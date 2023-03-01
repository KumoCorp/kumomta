use crate::egress_source::EgressSource;
use crate::lifecycle::{Activity, ShutdownSubcription};
use crate::logging::{log_disposition, RecordType};
use crate::mx::{MailExchanger, ResolvedAddress};
use crate::queue::{Queue, QueueManager};
use crate::spool::SpoolManager;
use anyhow::Context;
use config::load_config;
use message::message::QueueNameComponents;
use message::Message;
use mlua::prelude::*;
use prometheus::{IntCounter, IntGauge};
use rfc5321::{
    ClientError, EnhancedStatusCode, ForwardPath, Response, ReversePath, SmtpClient,
    SmtpClientTimeouts,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use throttle::ThrottleSpec;
use tokio::net::TcpSocket;
use tokio::sync::{Mutex, MutexGuard, Notify};
use tokio::task::JoinHandle;
use tokio::time::timeout;

lazy_static::lazy_static! {
    static ref MANAGER: Mutex<EgressPathManager> = Mutex::new(EgressPathManager::new());
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Copy)]
pub enum Tls {
    /// Use it if available. If the peer has invalid or self-signed certificates, then
    /// delivery will fail. Will NOT fallback to not using TLS if the peer advertises
    /// STARTTLS.
    Opportunistic,
    /// Use it if available, and allow self-signed or otherwise invalid server certs.
    /// Not recommended for sending to the public internet; this is for local/lab
    /// testing scenarios only.
    OpportunisticInsecure,
    /// TLS with valid certs is required.
    Required,
    /// Required, and allow self-signed or otherwise invalid server certs.
    /// Not recommended for sending to the public internet; this is for local/lab
    /// testing scenarios only.
    RequiredInsecure,
    /// Do not try to use TLS
    Disabled,
}

impl Tls {
    pub fn allow_insecure(&self) -> bool {
        match self {
            Self::OpportunisticInsecure | Self::RequiredInsecure => true,
            _ => false,
        }
    }
}

impl Default for Tls {
    fn default() -> Self {
        Self::Opportunistic
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct EgressPathConfig {
    #[serde(default = "EgressPathConfig::default_connection_limit")]
    connection_limit: usize,

    #[serde(default)]
    enable_tls: Tls,

    #[serde(flatten)]
    client_timeouts: SmtpClientTimeouts,

    #[serde(default = "EgressPathConfig::default_max_ready")]
    max_ready: usize,

    #[serde(default = "EgressPathConfig::default_consecutive_connection_failures_before_delay")]
    consecutive_connection_failures_before_delay: usize,

    #[serde(default = "EgressPathConfig::default_smtp_port")]
    smtp_port: u16,

    #[serde(default)]
    max_message_rate: Option<ThrottleSpec>,

    #[serde(default)]
    max_connection_rate: Option<ThrottleSpec>,

    #[serde(default)]
    max_deliveries_per_connection: Option<usize>,
}

impl LuaUserData for EgressPathConfig {}

impl Default for EgressPathConfig {
    fn default() -> Self {
        Self {
            connection_limit: Self::default_connection_limit(),
            enable_tls: Tls::default(),
            max_ready: Self::default_max_ready(),
            consecutive_connection_failures_before_delay:
                Self::default_consecutive_connection_failures_before_delay(),
            smtp_port: Self::default_smtp_port(),
            max_message_rate: None,
            max_connection_rate: None,
            max_deliveries_per_connection: None,
            client_timeouts: SmtpClientTimeouts::default(),
        }
    }
}

impl EgressPathConfig {
    fn default_connection_limit() -> usize {
        32
    }

    fn default_max_ready() -> usize {
        1024
    }

    fn default_consecutive_connection_failures_before_delay() -> usize {
        100
    }

    fn default_smtp_port() -> u16 {
        25
    }
}

pub struct EgressPathManager {
    paths: HashMap<String, EgressPathHandle>,
}

impl EgressPathManager {
    pub fn new() -> Self {
        Self {
            paths: HashMap::new(),
        }
    }

    pub fn number_of_sites(&self) -> usize {
        self.paths.len()
    }

    pub async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }

    pub async fn resolve_by_queue_name(
        queue_name: &str,
        egress_source: &str,
        egress_pool: &str,
    ) -> anyhow::Result<EgressPathHandle> {
        let components = QueueNameComponents::parse(queue_name);
        let mx = MailExchanger::resolve(components.domain).await?;
        let name = &mx.site_name;

        let name = format!("{egress_source}->{name}");
        let egress_source = EgressSource::resolve(egress_source)?;

        let mut config = load_config().await?;

        let path_config: EgressPathConfig = config.call_callback(
            "get_egress_path_config",
            (
                components.domain,
                egress_source.name.to_string(),
                name.to_string(),
            ),
        )?;

        let mut manager = Self::get().await;
        let activity = Activity::get()?;
        let handle = manager.paths.entry(name.clone()).or_insert_with(|| {
            tokio::spawn({
                let name = name.clone();
                async move {
                    let mut shutdown = ShutdownSubcription::get();
                    let mut interval = Duration::from_secs(60);
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(interval) => {},
                            _ = shutdown.shutting_down() => {
                                interval = Duration::from_secs(5);
                            }
                        };
                        let mut mgr = EgressPathManager::get().await;
                        let path = { mgr.paths.get(&name).cloned() };
                        match path {
                            None => break,
                            Some(path) => {
                                let mut path = path.lock().await;
                                if path.reapable() {
                                    tracing::debug!("reaping site {name}");
                                    mgr.paths.remove(&name);
                                    crate::metrics_helper::remove_metrics_for_service(&format!(
                                        "smtp_client:{name}"
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            });

            let service = format!("smtp_client:{name}");
            let metrics = DeliveryMetrics {
                connection_gauge: crate::metrics_helper::connection_gauge_for_service(&service),
                global_connection_gauge: crate::metrics_helper::connection_gauge_for_service(
                    "smtp_client",
                ),
                connection_total: crate::metrics_helper::connection_total_for_service(&service),
                global_connection_total: crate::metrics_helper::connection_total_for_service(
                    "smtp_client",
                ),
                msgs_delivered: crate::metrics_helper::total_msgs_delivered_for_service(&service),
                global_msgs_delivered: crate::metrics_helper::total_msgs_delivered_for_service(
                    "smtp_client",
                ),
                msgs_transfail: crate::metrics_helper::total_msgs_transfail_for_service(&service),
                global_msgs_transfail: crate::metrics_helper::total_msgs_transfail_for_service(
                    "smtp_client",
                ),
                msgs_fail: crate::metrics_helper::total_msgs_fail_for_service(&service),
                global_msgs_fail: crate::metrics_helper::total_msgs_fail_for_service("smtp_client"),
            };
            let ready = Arc::new(StdMutex::new(VecDeque::new()));
            let notify = Arc::new(Notify::new());
            EgressPathHandle(Arc::new(Mutex::new(EgressPath {
                name: name.clone(),
                ready,
                mx,
                notify,
                connections: vec![],
                last_change: Instant::now(),
                path_config,
                egress_source,
                metrics,
                activity,
                consecutive_connection_failures: Arc::new(AtomicUsize::new(0)),
                egress_pool: egress_pool.to_string(),
            })))
        });
        Ok(handle.clone())
    }
}

#[derive(Clone)]
pub struct EgressPathHandle(Arc<Mutex<EgressPath>>);

impl EgressPathHandle {
    pub async fn lock(&self) -> MutexGuard<EgressPath> {
        self.0.lock().await
    }
}

#[derive(Clone)]
struct DeliveryMetrics {
    connection_gauge: IntGauge,
    global_connection_gauge: IntGauge,
    connection_total: IntCounter,
    global_connection_total: IntCounter,

    msgs_delivered: IntCounter,
    global_msgs_delivered: IntCounter,

    msgs_transfail: IntCounter,
    global_msgs_transfail: IntCounter,

    msgs_fail: IntCounter,
    global_msgs_fail: IntCounter,
}

pub struct EgressPath {
    name: String,
    mx: Arc<MailExchanger>,
    ready: Arc<StdMutex<VecDeque<Message>>>,
    notify: Arc<Notify>,
    connections: Vec<JoinHandle<()>>,
    last_change: Instant,
    path_config: EgressPathConfig,
    egress_pool: String,
    egress_source: EgressSource,
    metrics: DeliveryMetrics,
    activity: Activity,
    consecutive_connection_failures: Arc<AtomicUsize>,
}

impl EgressPath {
    #[allow(unused)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn insert(&mut self, msg: Message) -> Result<(), Message> {
        // TODO: shrink if ready size is too large
        self.ready.lock().unwrap().push_back(msg);
        self.notify.notify_waiters();
        self.maintain();
        self.last_change = Instant::now();

        Ok(())
    }

    pub fn ready_count(&self) -> usize {
        self.ready.lock().unwrap().len()
    }

    pub fn ideal_connection_count(&self) -> usize {
        if self.activity.is_shutting_down() {
            0
        } else {
            ideal_connection_count(self.ready_count(), self.path_config.connection_limit)
        }
    }

    pub fn maintain(&mut self) {
        // Prune completed connection tasks
        self.connections.retain(|handle| !handle.is_finished());

        if self.activity.is_shutting_down() {
            // We are shutting down; we want all messages to get saved.
            let msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
            if !msgs.is_empty() {
                let activity = self.activity.clone();
                tokio::spawn(async move {
                    for msg in msgs {
                        Queue::save_if_needed_and_log(&msg).await;
                        drop(msg);
                    }
                    drop(activity);
                });
            }

            return;
        }

        // TODO: throttle rate at which connections are opened
        let ideal = self.ideal_connection_count();

        for _ in self.connections.len()..ideal {
            // Open a new connection
            let name = self.name.clone();
            let mx = self.mx.clone();
            let ready = Arc::clone(&self.ready);
            let notify = self.notify.clone();
            let path_config = self.path_config.clone();
            let metrics = self.metrics.clone();
            let egress_source = self.egress_source.clone();
            let egress_pool = self.egress_pool.clone();
            let consecutive_connection_failures = self.consecutive_connection_failures.clone();
            self.connections.push(tokio::spawn(async move {
                if let Err(err) = Dispatcher::run(
                    &name,
                    mx,
                    ready,
                    notify,
                    path_config,
                    metrics,
                    consecutive_connection_failures.clone(),
                    egress_source,
                    egress_pool,
                )
                .await
                {
                    tracing::debug!(
                        "Error in dispatch_queue for {name}: {err:#} \
                         (consecutive_connection_failures={consecutive_connection_failures:?})"
                    );
                }
            }));
        }
    }

    pub fn reapable(&mut self) -> bool {
        self.maintain();
        let ideal = self.ideal_connection_count();
        ideal == 0
            && self.connections.is_empty()
            && (self.last_change.elapsed() > Duration::from_secs(10 * 60))
                | self.activity.is_shutting_down()
    }
}

struct Dispatcher {
    name: String,
    ready: Arc<StdMutex<VecDeque<Message>>>,
    notify: Arc<Notify>,
    addresses: Vec<ResolvedAddress>,
    msg: Option<Message>,
    client: Option<SmtpClient>,
    client_address: Option<ResolvedAddress>,
    ehlo_name: String,
    path_config: EgressPathConfig,
    metrics: DeliveryMetrics,
    shutting_down: ShutdownSubcription,
    activity: Activity,
    egress_source: EgressSource,
    egress_pool: String,
    delivered_this_connection: usize,
}

impl Dispatcher {
    async fn run(
        name: &str,
        mx: Arc<MailExchanger>,
        ready: Arc<StdMutex<VecDeque<Message>>>,
        notify: Arc<Notify>,
        path_config: EgressPathConfig,
        metrics: DeliveryMetrics,
        consecutive_connection_failures: Arc<AtomicUsize>,
        egress_source: EgressSource,
        egress_pool: String,
    ) -> anyhow::Result<()> {
        let ehlo_name = gethostname::gethostname()
            .to_str()
            .unwrap_or("[127.0.0.1]")
            .to_string();

        let activity = Activity::get()?;

        let addresses = mx.resolve_addresses().await;
        let mut dispatcher = Self {
            name: name.to_string(),
            ready,
            notify,
            msg: None,
            client: None,
            client_address: None,
            addresses,
            ehlo_name,
            path_config,
            metrics,
            shutting_down: ShutdownSubcription::get(),
            activity,
            egress_source,
            egress_pool,
            delivered_this_connection: 0,
        };

        dispatcher.obtain_message();
        if dispatcher.msg.is_none() {
            // We raced with another dispatcher and there is no
            // more work to be done; no need to open a new connection.
            return Ok(());
        }

        loop {
            if !dispatcher.wait_for_message().await? {
                // No more messages within our idle time; we can close
                // the connection
                tracing::debug!("{} Idling out connection", dispatcher.name);
                return Ok(());
            }
            if let Err(err) = dispatcher.attempt_connection().await {
                dispatcher.metrics.connection_gauge.dec();
                dispatcher.metrics.global_connection_gauge.dec();
                if dispatcher.addresses.is_empty() {
                    if consecutive_connection_failures.fetch_add(1, Ordering::SeqCst)
                        > dispatcher
                            .path_config
                            .consecutive_connection_failures_before_delay
                    {
                        dispatcher.delay_ready_queue();
                    }
                    return Err(err);
                }
                tracing::debug!("{err:#}");
                // Try the next candidate MX address
                continue;
            }
            consecutive_connection_failures.store(0, Ordering::SeqCst);
            dispatcher.deliver_message().await?;
        }
    }

    fn throttle_ready_queue(&mut self, delay: Duration) {
        let mut msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
        if let Some(msg) = self.msg.take() {
            msgs.push(msg);
        }
        if !msgs.is_empty() {
            tracing::debug!(
                "throttled: delaying ready queue {} - {} messages",
                self.name,
                msgs.len()
            );
            let activity = self.activity.clone();
            let delay = chrono::Duration::from_std(delay).unwrap_or_else(|err| {
                tracing::error!(
                    "error creating duration from {delay:?}: {err:#}. Using 1 minute instead"
                );
                chrono::Duration::seconds(60)
            });
            tokio::spawn(async move {
                for msg in msgs {
                    if let Err(err) = Self::requeue_message(msg, false, Some(delay)).await {
                        tracing::error!("error requeuing message: {err:#}");
                    }
                }
                drop(activity);
            });
        }
    }

    fn delay_ready_queue(&mut self) {
        let mut msgs: Vec<Message> = self.ready.lock().unwrap().drain(..).collect();
        if let Some(msg) = self.msg.take() {
            msgs.push(msg);
        }
        if !msgs.is_empty() {
            tracing::debug!(
                "too many connection failures, delaying ready queue {} - {} messages",
                self.name,
                msgs.len()
            );
            let activity = self.activity.clone();
            let name = self.name.clone();
            let egress_pool = self.egress_pool.clone();
            let egress_source = self.egress_source.name.clone();
            tokio::spawn(async move {
                let increment_attempts = true;
                for msg in msgs {
                    log_disposition(
                        RecordType::Delivery,
                        msg.clone(),
                        &name,
                        None,
                        Response {
                            code: 451,
                            enhanced_code: Some(EnhancedStatusCode {
                                class: 4,
                                subject: 4,
                                detail: 1,
                            }),
                            content: "No answer from any hosts listed in MX".to_string(),
                            command: None,
                        },
                        Some(&egress_pool),
                        Some(&egress_source),
                    )
                    .await;

                    if let Err(err) = Self::requeue_message(msg, increment_attempts, None).await {
                        tracing::error!("error requeuing message: {err:#}");
                    }
                }
                drop(activity);
            });
        }
    }

    fn obtain_message(&mut self) -> bool {
        if self.msg.is_some() {
            return true;
        }
        self.msg = self.ready.lock().unwrap().pop_front();
        self.msg.is_some()
    }

    async fn wait_for_message(&mut self) -> anyhow::Result<bool> {
        if self.activity.is_shutting_down() {
            if let Some(msg) = self.msg.take() {
                Queue::save_if_needed_and_log(&msg).await;
            }
            return Ok(false);
        }

        if let Some(limit) = self.path_config.max_deliveries_per_connection {
            if self.delivered_this_connection >= limit {
                if let Some(mut client) = self.client.take() {
                    tracing::trace!(
                        "Sent {} and limit is {limit}, close and make a new connection",
                        self.delivered_this_connection
                    );
                    client.send_command(&rfc5321::Command::Quit).await.ok();
                    // Close out this dispatcher and let the maintainer spawn
                    // a new connection
                    return Ok(false);
                }
            }
        }

        if self.obtain_message() {
            return Ok(true);
        }

        let idle_timeout = self.path_config.client_timeouts.idle_timeout;
        tokio::select! {
            _ = tokio::time::sleep(idle_timeout) => {},
            _ = self.notify.notified() => {}
            _ = self.shutting_down.shutting_down() => {
                return Ok(false);
            }
        };
        Ok(self.obtain_message())
    }

    async fn attempt_connection(&mut self) -> anyhow::Result<()> {
        if self.client.is_some() {
            return Ok(());
        }

        if let Some(throttle) = &self.path_config.max_connection_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-connection-rate", self.name))
                    .await?;

                if let Some(delay) = result.retry_after {
                    if delay >= self.path_config.client_timeouts.idle_timeout {
                        self.throttle_ready_queue(delay);
                        anyhow::bail!("connection rate throttled for {delay:?}");
                    }
                    tracing::trace!(
                        "{} throttled connection rate, sleep for {delay:?}",
                        self.name
                    );
                    let mut shutdown = ShutdownSubcription::get();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = shutdown.shutting_down() => {
                            anyhow::bail!("shutting down");
                        }
                    };
                } else {
                    break;
                }
            }
        }

        self.metrics.connection_gauge.inc();
        self.metrics.global_connection_gauge.inc();
        self.metrics.connection_total.inc();
        self.metrics.global_connection_total.inc();

        let address = self
            .addresses
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no more addresses to try!"))?;

        let ehlo_name = self.ehlo_name.to_string();
        let mx_host = address.name.to_string();
        let enable_tls = self.path_config.enable_tls;
        let egress_source_name = self.egress_source.name.to_string();
        let port = self
            .egress_source
            .remote_port
            .unwrap_or(self.path_config.smtp_port);
        let source_address = self.egress_source.source_address.clone();

        let mut client = timeout(self.path_config.client_timeouts.connect_timeout, {
            let address = address.clone();
            let timeouts = self.path_config.client_timeouts.clone();
            async move {
                let socket = match address.addr {
                    IpAddr::V4(_) => TcpSocket::new_v4(),
                    IpAddr::V6(_) => TcpSocket::new_v6(),
                }
                .with_context(|| format!("make socket to connect to {address:?} port {port}"))?;
                if let Some(source) = source_address {
                    if let Err(err) = socket.bind(SocketAddr::new(source, 0)) {
                        // Always log failure to bind: it indicates a critical
                        // misconfiguration issue
                        let error = format!(
                            "bind {source:?} for source:{egress_source_name} failed: {err:#} \
                             while attempting to connect to {address:?} port {port}"
                        );
                        tracing::error!("{error}");
                        anyhow::bail!("{error}");
                    }
                }
                let stream = socket.connect(SocketAddr::new(address.addr, port)).await?;

                let mut client = SmtpClient::with_stream(stream, &mx_host, timeouts);

                // Read banner
                let banner = client.read_response(None).await?;
                if banner.code != 220 {
                    return Err(ClientError::Rejected(banner).into());
                }

                Ok(client)
            }
        })
        .await
        .with_context(|| format!("connect to {address:?} port {port} and read initial banner"))??;

        // Say EHLO
        let caps = client.ehlo(&ehlo_name).await?;

        // Use STARTTLS if available.
        let has_tls = caps.contains_key("STARTTLS");
        match (enable_tls, has_tls) {
            (Tls::Required | Tls::RequiredInsecure, false) => {
                anyhow::bail!("tls policy is {enable_tls:?} but STARTTLS is not advertised",);
            }
            (Tls::Disabled, _) | (Tls::Opportunistic | Tls::OpportunisticInsecure, false) => {
                // Do not use TLS
            }
            (
                Tls::Opportunistic
                | Tls::OpportunisticInsecure
                | Tls::Required
                | Tls::RequiredInsecure,
                true,
            ) => {
                client.starttls(enable_tls.allow_insecure()).await?;
            }
        };

        self.client.replace(client);
        self.client_address.replace(address);
        self.delivered_this_connection = 0;
        Ok(())
    }

    async fn requeue_message(
        msg: Message,
        increment_attempts: bool,
        delay: Option<chrono::Duration>,
    ) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let queue_name = msg.get_queue_name()?;
        let queue = QueueManager::resolve(&queue_name).await?;
        let mut queue = queue.lock().await;
        queue.requeue_message(msg, increment_attempts, delay).await
    }

    async fn deliver_message(&mut self) -> anyhow::Result<()> {
        let data;
        let sender: ReversePath;
        let recipient: ForwardPath;

        {
            let msg = self.msg.as_ref().unwrap();

            msg.load_meta_if_needed().await?;
            msg.load_data_if_needed().await?;

            data = msg.get_data();
            sender = msg
                .sender()?
                .try_into()
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            recipient = msg
                .recipient()?
                .try_into()
                .map_err(|err| anyhow::anyhow!("{err}"))?;
        }

        let activity = match Activity::get_opt() {
            Some(a) => a,
            None => {
                return Ok(());
            }
        };

        if let Some(throttle) = &self.path_config.max_message_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-message-rate", self.name))
                    .await?;

                if let Some(delay) = result.retry_after {
                    if delay >= self.path_config.client_timeouts.idle_timeout {
                        self.throttle_ready_queue(delay);
                        return Ok(());
                    }
                    tracing::trace!("{} throttled message rate, sleep for {delay:?}", self.name);
                    let mut shutdown = ShutdownSubcription::get();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = shutdown.shutting_down() => {
                            anyhow::bail!("shutting down");
                        }
                    };
                } else {
                    break;
                }
            }
        }

        self.delivered_this_connection += 1;
        match self
            .client
            .as_mut()
            .unwrap()
            .send_mail(sender, recipient, &*data)
            .await
        {
            Err(ClientError::Rejected(response)) if response.code >= 400 && response.code < 500 => {
                // Transient failure
                tracing::debug!(
                    "failed to send message to {} {:?}: {response:?}",
                    self.name,
                    self.client_address
                );
                if let Some(msg) = self.msg.take() {
                    log_disposition(
                        RecordType::Delivery,
                        msg.clone(),
                        &self.name,
                        self.client_address.as_ref(),
                        response,
                        Some(&self.egress_pool),
                        Some(&self.egress_source.name),
                    )
                    .await;
                    tokio::spawn(async move { Self::requeue_message(msg, true, None).await });
                }
                self.metrics.msgs_transfail.inc();
                self.metrics.global_msgs_transfail.inc();
            }
            Err(ClientError::Rejected(response)) => {
                self.metrics.msgs_fail.inc();
                self.metrics.global_msgs_fail.inc();
                tracing::debug!(
                    "failed to send message to {} {:?}: {response:?}",
                    self.name,
                    self.client_address
                );
                if let Some(msg) = self.msg.take() {
                    log_disposition(
                        RecordType::Delivery,
                        msg.clone(),
                        &self.name,
                        self.client_address.as_ref(),
                        response,
                        Some(&self.egress_pool),
                        Some(&self.egress_source.name),
                    )
                    .await;
                    tokio::spawn(async move { SpoolManager::remove_from_spool(*msg.id()).await });
                }
            }
            Err(err) => {
                // Transient failure; continue with another host
                tracing::debug!(
                    "failed to send message to {} {:?}: {err:#}",
                    self.name,
                    self.client_address
                );
                return Err(err.into());
            }
            Ok(response) => {
                tracing::debug!("Delivered OK! {response:?}");
                if let Some(msg) = self.msg.take() {
                    log_disposition(
                        RecordType::Delivery,
                        msg.clone(),
                        &self.name,
                        self.client_address.as_ref(),
                        response,
                        Some(&self.egress_pool),
                        Some(&self.egress_source.name),
                    )
                    .await;
                    tokio::spawn(async move { SpoolManager::remove_from_spool(*msg.id()).await });
                }
                self.metrics.msgs_delivered.inc();
                self.metrics.global_msgs_delivered.inc();
            }
        };

        drop(activity);

        Ok(())
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        // Ensure that we re-queue any message that we had popped
        if let Some(msg) = self.msg.take() {
            let activity = self.activity.clone();
            tokio::spawn(async move {
                if activity.is_shutting_down() {
                    Queue::save_if_needed_and_log(&msg).await;
                } else if let Err(err) = Dispatcher::requeue_message(msg, false, None).await {
                    tracing::error!("error requeuing message: {err:#}");
                }
            });
        }
        if self.client.is_some() {
            self.metrics.connection_gauge.dec();
            self.metrics.global_connection_gauge.dec();
        }
    }
}

/// Use an exponential decay curve in the increasing form, asymptotic up to connection_limit,
/// passes through 0.0, increasing but bounded to connection_limit.
///
/// Visualize on wolframalpha: "plot 32 * (1-exp(-x * 0.023)), x from 0 to 100, y from 0 to 32"
fn ideal_connection_count(queue_size: usize, connection_limit: usize) -> usize {
    let factor = 0.023;
    let goal = (connection_limit as f32) * (1. - (-1.0 * queue_size as f32 * factor).exp());
    goal.ceil() as usize
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn connection_limit() {
        let sizes = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 32, 64, 128, 256, 400, 512, 1024,
        ];
        let max_connections = 32;
        let targets: Vec<(usize, usize)> = sizes
            .iter()
            .map(|&queue_size| {
                (
                    queue_size,
                    ideal_connection_count(queue_size, max_connections),
                )
            })
            .collect();
        assert_eq!(
            vec![
                (0, 0),
                (1, 1),
                (2, 2),
                (3, 3),
                (4, 3),
                (5, 4),
                (6, 5),
                (7, 5),
                (8, 6),
                (9, 6),
                (10, 7),
                (20, 12),
                (32, 17),
                (64, 25),
                (128, 31),
                (256, 32),
                (400, 32),
                (512, 32),
                (1024, 32)
            ],
            targets
        );
    }
}
