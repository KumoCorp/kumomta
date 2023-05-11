use crate::delivery_metrics::MetricsWrappedConnection;
use crate::egress_path::Tls;
use crate::lifecycle::ShutdownSubcription;
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::runtime::{rt_spawn, spawn};
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use kumo_log_types::ResolvedAddress;
use message::Message;
use rfc5321::{ClientError, EnhancedStatusCode, ForwardPath, Response, ReversePath, SmtpClient};
use std::net::SocketAddr;
use tokio::time::timeout;

#[derive(Debug)]
pub struct SmtpDispatcher {
    addresses: Vec<ResolvedAddress>,
    client: Option<MetricsWrappedConnection<SmtpClient>>,
    client_address: Option<ResolvedAddress>,
    ehlo_name: String,
}

impl SmtpDispatcher {
    pub async fn init(dispatcher: &mut Dispatcher) -> anyhow::Result<Option<Self>> {
        let ehlo_name = match &dispatcher.path_config.ehlo_domain {
            Some(n) => n.to_string(),
            None => gethostname::gethostname()
                .to_str()
                .unwrap_or("[127.0.0.1]")
                .to_string(),
        };

        let mut addresses = dispatcher
            .mx
            .as_ref()
            .expect("to have mx when doing smtp")
            .resolve_addresses()
            .await;
        tracing::trace!("mx resolved to {addresses:?}");

        if addresses.is_empty() {
            dispatcher
                .bulk_ready_queue_operation(Response {
                    code: 556,
                    enhanced_code: Some(EnhancedStatusCode {
                        class: 5,
                        subject: 1,
                        detail: 10,
                    }),
                    content: "Recipient address has a null MX".to_string(),
                    command: None,
                })
                .await;
            return Ok(None);
        }

        for addr in &addresses {
            if dispatcher.path_config.prohibited_hosts.contains(addr.addr) {
                dispatcher
                    .bulk_ready_queue_operation(Response {
                        code: 550,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 5,
                            subject: 4,
                            detail: 4,
                        }),
                        content: format!(
                            "{addr:?} is on the list of prohibited_hosts {:?}",
                            dispatcher.path_config.prohibited_hosts
                        ),
                        command: None,
                    })
                    .await;
                return Ok(None);
            }
        }

        addresses.retain(|addr| !dispatcher.path_config.skip_hosts.contains(addr.addr));

        if addresses.is_empty() {
            dispatcher
                .bulk_ready_queue_operation(Response {
                    code: 550,
                    enhanced_code: Some(EnhancedStatusCode {
                        class: 5,
                        subject: 4,
                        detail: 4,
                    }),
                    content: "MX consisted solely of hosts on the skip_hosts list".to_string(),
                    command: None,
                })
                .await;
            return Ok(None);
        }

        Ok(Some(Self {
            addresses,
            client: None,
            client_address: None,
            ehlo_name,
        }))
    }
}

#[async_trait(?Send)]
impl QueueDispatcher for SmtpDispatcher {
    async fn close_connection(&mut self, _dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        if let Some(mut client) = self.client.take() {
            client.send_command(&rfc5321::Command::Quit).await.ok();
            // Close out this dispatcher and let the maintainer spawn
            // a new connection
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        if self.client.is_some() {
            return Ok(());
        }

        if let Some(throttle) = &dispatcher.path_config.max_connection_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-connection-rate", dispatcher.name))
                    .await?;

                if let Some(delay) = result.retry_after {
                    if delay >= dispatcher.path_config.client_timeouts.idle_timeout {
                        dispatcher.throttle_ready_queue(delay).await;
                        anyhow::bail!("connection rate throttled for {delay:?}");
                    }
                    tracing::trace!(
                        "{} throttled connection rate, sleep for {delay:?}",
                        dispatcher.name
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

        let connection_wrapper = dispatcher.metrics.wrap_connection(());

        let address = self
            .addresses
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no more addresses to try!"))?;

        let ehlo_name = self.ehlo_name.to_string();
        let mx_host = address.name.to_string();
        let enable_tls = dispatcher.path_config.enable_tls;
        let port = dispatcher
            .egress_source
            .remote_port
            .unwrap_or(dispatcher.path_config.smtp_port);
        let connect_context = format!("connect to {address:?} port {port} and read initial banner");

        let mut client = timeout(dispatcher.path_config.client_timeouts.connect_timeout, {
            let address = address.clone();
            let timeouts = dispatcher.path_config.client_timeouts.clone();
            let egress_source = dispatcher.egress_source.clone();
            async move {
                let (stream, source_address) = egress_source
                    .connect_to(SocketAddr::new(address.addr, port))
                    .await?;

                tracing::debug!(
                    "connected to {address:?} port {port} via source address {source_address:?}"
                );

                let mut client = SmtpClient::with_stream(stream, &mx_host, timeouts);

                // Read banner
                let banner = client.read_response(None).await.context("reading banner")?;
                if banner.code != 220 {
                    return anyhow::Result::<SmtpClient>::Err(ClientError::Rejected(banner).into());
                }

                Ok(client)
            }
        })
        .await
        .with_context(|| connect_context.clone())?
        .with_context(|| connect_context.clone())?;

        // Say EHLO
        let caps = client.ehlo(&ehlo_name).await.context("EHLO")?;

        // Use STARTTLS if available.
        let has_tls = caps.contains_key("STARTTLS");
        let tls_enabled = match (enable_tls, has_tls) {
            (Tls::Required | Tls::RequiredInsecure, false) => {
                anyhow::bail!("tls policy is {enable_tls:?} but STARTTLS is not advertised",);
            }
            (Tls::Disabled, _) | (Tls::Opportunistic | Tls::OpportunisticInsecure, false) => {
                // Do not use TLS
                false
            }
            (
                Tls::Opportunistic
                | Tls::OpportunisticInsecure
                | Tls::Required
                | Tls::RequiredInsecure,
                true,
            ) => {
                if let Some(handshake_error) = client.starttls(enable_tls.allow_insecure()).await? {
                    client.send_command(&rfc5321::Command::Quit).await.ok();
                    anyhow::bail!("TLS handshake failed: {handshake_error}");
                }
                true
            }
        };

        if let Some(username) = &dispatcher.path_config.smtp_auth_plain_username {
            if !tls_enabled && !dispatcher.path_config.allow_smtp_auth_plain_without_tls {
                anyhow::bail!("TLS is not enabled and AUTH PLAIN is required. Skipping this host");
            }

            let password = if let Some(pw) = &dispatcher.path_config.smtp_auth_plain_password {
                Some(
                    String::from_utf8(
                        pw.get()
                            .await
                            .context("fetching smtp_auth_plain_password")?,
                    )
                    .context("smtp_auth_plain_password is not UTF8")?,
                )
            } else {
                None
            };

            client
                .auth_plain(username, password.as_deref())
                .await
                .with_context(|| format!("authenticating as {username} via SMTP AUTH PLAIN"))?;
        }

        self.client
            .replace(connection_wrapper.map_connection(client));
        self.client_address.replace(address);
        dispatcher.delivered_this_connection = 0;
        Ok(())
    }

    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        !self.addresses.is_empty()
    }

    async fn deliver_message(
        &mut self,
        msg: Message,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        msg.load_meta_if_needed().await?;
        msg.load_data_if_needed().await?;

        let data = msg.get_data();
        let sender: ReversePath = msg
            .sender()?
            .try_into()
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let recipient: ForwardPath = msg
            .recipient()?
            .try_into()
            .map_err(|err| anyhow::anyhow!("{err}"))?;

        dispatcher.delivered_this_connection += 1;
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
                    dispatcher.name,
                    self.client_address
                );
                if let Some(msg) = dispatcher.msg.take() {
                    log_disposition(LogDisposition {
                        kind: RecordType::TransientFailure,
                        msg: msg.clone(),
                        site: &dispatcher.name,
                        peer_address: self.client_address.as_ref(),
                        response,
                        egress_pool: Some(&dispatcher.egress_pool),
                        egress_source: Some(&dispatcher.egress_source.name),
                        relay_disposition: None,
                        delivery_protocol: Some(&dispatcher.delivery_protocol),
                    })
                    .await;
                    rt_spawn("requeue message".to_string(), move || {
                        Ok(async move { Dispatcher::requeue_message(msg, true, None).await })
                    })
                    .await?;
                }
                dispatcher.metrics.msgs_transfail.inc();
                dispatcher.metrics.global_msgs_transfail.inc();
            }
            Err(ClientError::Rejected(response)) => {
                dispatcher.metrics.msgs_fail.inc();
                dispatcher.metrics.global_msgs_fail.inc();
                tracing::debug!(
                    "failed to send message to {} {:?}: {response:?}",
                    dispatcher.name,
                    self.client_address
                );
                if let Some(msg) = dispatcher.msg.take() {
                    log_disposition(LogDisposition {
                        kind: RecordType::Bounce,
                        msg: msg.clone(),
                        site: &dispatcher.name,
                        peer_address: self.client_address.as_ref(),
                        response,
                        egress_pool: Some(&dispatcher.egress_pool),
                        egress_source: Some(&dispatcher.egress_source.name),
                        relay_disposition: None,
                        delivery_protocol: Some(&dispatcher.delivery_protocol),
                    })
                    .await;
                    spawn("remove from spool", async move {
                        SpoolManager::remove_from_spool(*msg.id()).await
                    })?;
                }
            }
            Err(err) => {
                // Transient failure; continue with another host
                tracing::debug!(
                    "failed to send message to {} {:?}: {err:#}",
                    dispatcher.name,
                    self.client_address
                );
                return Err(err.into());
            }
            Ok(response) => {
                tracing::debug!("Delivered OK! {response:?}");
                if let Some(msg) = dispatcher.msg.take() {
                    log_disposition(LogDisposition {
                        kind: RecordType::Delivery,
                        msg: msg.clone(),
                        site: &dispatcher.name,
                        peer_address: self.client_address.as_ref(),
                        response,
                        egress_pool: Some(&dispatcher.egress_pool),
                        egress_source: Some(&dispatcher.egress_source.name),
                        relay_disposition: None,
                        delivery_protocol: Some(&dispatcher.delivery_protocol),
                    })
                    .await;
                    spawn("remove from spool", async move {
                        SpoolManager::remove_from_spool(*msg.id()).await
                    })?;
                }
                dispatcher.metrics.msgs_delivered.inc();
                dispatcher.metrics.global_msgs_delivered.inc();
            }
        };

        Ok(())
    }
}
