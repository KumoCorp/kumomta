use crate::delivery_metrics::MetricsWrappedConnection;
use crate::http_server::admin_trace_smtp_client_v1::{
    SmtpClientTraceEventPayload, SmtpClientTracerImpl,
};
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::{IncrementAttempts, QueueManager};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use config::{load_config, CallbackSignature};
use dns_resolver::{resolve_a_or_aaaa, ResolvedMxAddresses};
use kumo_api_types::egress_path::{EgressPathConfig, Tls};
use kumo_log_types::{MaybeProxiedSourceAddress, ResolvedAddress};
use kumo_server_lifecycle::ShutdownSubcription;
use kumo_server_runtime::spawn;
use lruttl::LruCacheWithTtl;
use message::message::QueueNameComponents;
use message::Message;
use mta_sts::policy::PolicyMode;
use rfc5321::{
    ClientError, EnhancedStatusCode, ForwardPath, Response, ReversePath, SmtpClient,
    TlsInformation, TlsOptions, TlsStatus,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use tracing::Level;

static BROKEN_TLS_BY_SITE: LazyLock<LruCacheWithTtl<String, ()>> =
    LazyLock::new(|| LruCacheWithTtl::new_named("smtp_dispatcher_broken_tls", 64 * 1024));

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct SmtpProtocol {
    #[serde(default)]
    pub mx_list: Vec<MxListEntry>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum MxListEntry {
    /// A name that needs to be resolved to its A or AAAA record in DNS,
    /// or an IP domain literal enclosed in square brackets like `[10.0.0.1]`
    Name(String),
    /// A pre-resolved name and IP address
    Resolved(ResolvedAddress),
}

#[derive(Debug)]
pub struct SmtpDispatcher {
    addresses: Vec<ResolvedAddress>,
    client: Option<MetricsWrappedConnection<SmtpClient>>,
    client_address: Option<ResolvedAddress>,
    source_address: Option<MaybeProxiedSourceAddress>,
    ehlo_name: String,
    tls_info: Option<TlsInformation>,
    tracer: Arc<SmtpClientTracerImpl>,
}

#[derive(thiserror::Error, Debug)]
#[error("{address}: EHLO after OpportunisticInsecure STARTTLS handshake status: {label}")]
#[must_use]
pub struct OpportunisticInsecureTlsHandshakeError {
    pub error: ClientError,
    pub address: String,
    pub label: String,
}

impl OpportunisticInsecureTlsHandshakeError {
    pub fn is_match_anyhow(err: &anyhow::Error) -> bool {
        Self::is_match(err.root_cause())
    }

    pub fn is_match(err: &(dyn std::error::Error + 'static)) -> bool {
        if let Some(cause) = err.source() {
            return Self::is_match(cause);
        } else {
            err.downcast_ref::<Self>().is_some()
        }
    }
}

impl SmtpDispatcher {
    pub async fn init(
        dispatcher: &mut Dispatcher,
        proto_config: &SmtpProtocol,
    ) -> anyhow::Result<Option<Self>> {
        let path_config = dispatcher.path_config.borrow().clone();
        let ehlo_name = match &path_config.ehlo_domain {
            Some(n) => n.to_string(),
            None => match &dispatcher.egress_source.ehlo_domain {
                Some(n) => n.to_string(),
                None => gethostname::gethostname()
                    .to_str()
                    .unwrap_or("[127.0.0.1]")
                    .to_string(),
            },
        };

        let addresses = if proto_config.mx_list.is_empty() {
            dispatcher
                .mx
                .as_ref()
                .expect("to have mx when doing smtp")
                .resolve_addresses()
                .await
        } else {
            let mut addresses = vec![];
            for a in proto_config.mx_list.iter() {
                match a {
                    MxListEntry::Name(a) => {
                        addresses.append(
                            &mut resolve_a_or_aaaa(a)
                                .await
                                .with_context(|| format!("resolving mx_list entry {a}"))?,
                        );
                    }
                    MxListEntry::Resolved(addr) => {
                        addresses.append(&mut vec![addr.clone()]);
                    }
                }
            }
            ResolvedMxAddresses::Addresses(addresses)
        };

        let tracer = Arc::new(SmtpClientTracerImpl::new(serde_json::json!({
            "egress_pool": dispatcher.egress_pool.to_string(),
            "egress_source": dispatcher.egress_source.name.to_string(),
            "id": dispatcher.session_id.to_string(),
            "ready_queue_name": dispatcher.name.to_string(),
            "mx_plan": addresses.clone(),
        })));

        tracing::trace!("mx resolved to {addresses:?}");

        let mut addresses = match addresses {
            ResolvedMxAddresses::NullMx => {
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
            ResolvedMxAddresses::Addresses(a) => a,
        };

        if addresses.is_empty() {
            dispatcher
                .bulk_ready_queue_operation(Response {
                    code: 451,
                    enhanced_code: Some(EnhancedStatusCode {
                        class: 4,
                        subject: 4,
                        detail: 4,
                    }),
                    content: "MX didn't resolve to any hosts".to_string(),
                    command: None,
                })
                .await;
            return Ok(None);
        }

        for addr in &addresses {
            if path_config.prohibited_hosts.contains(addr.addr) {
                dispatcher
                    .bulk_ready_queue_operation(Response {
                        code: 550,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 5,
                            subject: 4,
                            detail: 4,
                        }),
                        content: format!(
                            "{addr} is on the list of prohibited_hosts {:?}",
                            path_config.prohibited_hosts
                        ),
                        command: None,
                    })
                    .await;
                return Ok(None);
            }
        }

        addresses.retain(|addr| !path_config.skip_hosts.contains(addr.addr));

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
            tls_info: None,
            source_address: None,
            tracer,
        }))
    }

    async fn attempt_connection_impl(
        &mut self,
        dispatcher: &mut Dispatcher,
        tls_override: Option<Tls>,
    ) -> anyhow::Result<()> {
        if self.client.is_some() {
            return Ok(());
        }

        let mut shutdown = ShutdownSubcription::get();

        let path_config = dispatcher.path_config.borrow();
        if let Some(throttle) = path_config.max_connection_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-connection-rate", dispatcher.name))
                    .await?;

                if let Some(delay) = result.retry_after {
                    if delay >= path_config.client_timeouts.idle_timeout {
                        self.tracer.diagnostic(Level::INFO, || {
                            format!(
                                "Connection rate throttled for {delay:?} which is \
                                 longer than the idle_timeout, will disconnect"
                            )
                        });
                        dispatcher.throttle_ready_queue(delay).await;
                        anyhow::bail!("connection rate throttled for {delay:?}");
                    }
                    self.tracer.diagnostic(Level::INFO, || {
                        format!("Connection rate throttled for {delay:?}, pausing.")
                    });
                    tracing::trace!(
                        "{} throttled connection rate, sleep for {delay:?}",
                        dispatcher.name
                    );
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

        // This pops the next address (which is at the end) from the
        // list of candidate addresses.
        // Be aware that in the failed TLS handshake case below,
        // the current address is put back before we recurse to
        // try again.
        let address = self
            .addresses
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no more addresses to try!"))?;

        let ehlo_name = self.ehlo_name.to_string();
        let mx_host = address.name.to_string();
        let mut enable_tls = tls_override.unwrap_or(path_config.enable_tls);
        let port = dispatcher
            .egress_source
            .remote_port
            .unwrap_or(path_config.smtp_port);
        let connect_context = format!("connect to {address} port {port} and read initial banner");

        self.tracer.diagnostic(Level::INFO, || {
            format!("Attempting connection to {address} port {port}")
        });

        let make_connection = {
            let address = address.clone();
            let timeouts = path_config.client_timeouts.clone();
            let egress_source = dispatcher.egress_source.clone();
            let tracer = self.tracer.clone();

            // We need to spawn the connection attempt into another task,
            // otherwise the select! invocation below won't run it in parallel with
            // awaiting the shutdown subscription, causing us to uselessly wait
            // for the full connect timeout during shutdown.
            tokio::spawn(async move {
                let (stream, source_address) = egress_source
                    .connect_to(
                        SocketAddr::new(address.addr, port),
                        timeouts.connect_timeout,
                    )
                    .await?;

                tracing::debug!(
                    "connected to {address} port {port} via source address {source_address:?}"
                );

                let mut client = SmtpClient::with_stream(stream, &mx_host, timeouts);
                tracer.set_meta("source_address", source_address.address.to_string());
                tracer.set_meta("mx_host", mx_host.to_string());
                tracer.set_meta("mx_address", address.addr.to_string());
                tracer.submit(|| SmtpClientTraceEventPayload::Connected);

                client.set_tracer(tracer);

                // Read banner
                let banner = client
                    .read_response(None, timeouts.banner_timeout)
                    .await
                    .context("reading banner")?;
                if banner.code != 220 {
                    return anyhow::Result::<(SmtpClient, MaybeProxiedSourceAddress)>::Err(
                        ClientError::Rejected(banner).into(),
                    );
                }

                Ok((client, source_address))
            })
        };

        self.source_address.take();
        let (mut client, source_address) = tokio::select! {
            _ = shutdown.shutting_down() => {
                anyhow::bail!("shutting down");
            }
            result = make_connection => { result? },
        }
        .with_context(|| connect_context.clone())?;
        self.source_address.replace(source_address);

        // Say EHLO/LHLO
        let helo_verb = if path_config.use_lmtp { "LHLO" } else { "EHLO" };
        let pretls_caps = client
            .ehlo_lhlo(&ehlo_name, path_config.use_lmtp)
            .await
            .with_context(|| format!("{address}:{port}: {helo_verb} after banner"))?;

        // Use STARTTLS if available.
        let has_tls = pretls_caps.contains_key("STARTTLS");
        let broken_tls = self.has_broken_tls(&dispatcher.name, &path_config);

        let mut dane_tlsa = vec![];
        let mut mta_sts_eligible = true;

        let openssl_options = path_config.openssl_options.clone();
        let openssl_cipher_list = path_config.openssl_cipher_list.clone();
        let openssl_cipher_suites = path_config.openssl_cipher_suites.clone();
        let rustls_cipher_suites = path_config.rustls_cipher_suites.clone();

        if path_config.enable_dane {
            if let Some(mx) = &dispatcher.mx {
                match dns_resolver::resolve_dane(&mx.domain_name, port).await {
                    Ok(tlsa) => {
                        dane_tlsa = tlsa;
                        self.tracer.diagnostic(Level::INFO, || {
                            format!("DANE records for {} are: {dane_tlsa:?}", mx.domain_name)
                        });
                        if !dane_tlsa.is_empty() {
                            enable_tls = Tls::Required;
                            // Do not use MTA-STS when there are DANE results
                            mta_sts_eligible = false;
                        }
                    }
                    Err(err) => {
                        // Do not use MTA-STS when DANE results have been tampered
                        mta_sts_eligible = false;
                        self.tracer.diagnostic(Level::INFO, || {
                            format!("DANE resolve error for {}: {err:#}", mx.domain_name)
                        });
                        tracing::error!("DANE result for {}: {err:#}", mx.domain_name);
                        // TODO: should we prevent continuing in the clear here? probably
                    }
                }
            } else {
                self.tracer.diagnostic(Level::INFO, || {
                    format!(
                        "DANE is enabled for this path, but it is not using MX records from DNS"
                    )
                });
            }
        } else {
            self.tracer
                .diagnostic(Level::INFO, || format!("DANE is not enabled for this path"));
        }

        // Figure out MTA-STS policy.
        if mta_sts_eligible && path_config.enable_mta_sts {
            if let Some(mx) = &dispatcher.mx {
                match mta_sts::get_policy_for_domain(&mx.domain_name).await {
                    Ok(policy) => {
                        self.tracer.diagnostic(Level::INFO, || {
                            format!("MTA-STS policy for {} is {:?}", mx.domain_name, policy.mode)
                        });

                        match policy.mode {
                            PolicyMode::Enforce => {
                                enable_tls = Tls::Required;
                                if !policy.mx_name_matches(&address.name) {
                                    anyhow::bail!(
                                        "MTA-STS policy for {domain} is set to \
                                     enforce but the current MX candidate \
                                     {mx_host} does not match the list of allowed \
                                     hosts. {policy:?}",
                                        domain = mx.domain_name,
                                        mx_host = address.name
                                    );
                                }
                            }
                            PolicyMode::Testing => {
                                enable_tls = Tls::OpportunisticInsecure;
                            }
                            PolicyMode::None => {}
                        }
                    }
                    Err(err) => {
                        self.tracer.diagnostic(Level::INFO, || {
                            format!("MTA-STS resolve error for {}: {err:#}", mx.domain_name)
                        });
                    }
                }
            } else {
                self.tracer.diagnostic(Level::INFO, || {
                    format!(
                        "MTA-STS is enabled for this path, but it is not using MX records from DNS"
                    )
                });
            }
        } else {
            self.tracer.diagnostic(Level::INFO, || {
                format!("MTA-STS is not enabled for this path")
            });
        }

        let prefer_openssl = path_config.tls_prefer_openssl;

        // A couple of little helper types to make the match statement below
        // a bit easier to grok at a glance
        enum AdvTls {
            Yes,
            No,
        }

        enum BrokenTls {
            Yes,
            No,
        }

        let has_tls = if has_tls { AdvTls::Yes } else { AdvTls::No };

        let broken_tls = if broken_tls {
            BrokenTls::Yes
        } else {
            BrokenTls::No
        };

        let tls_enabled = match (enable_tls, has_tls, broken_tls) {
            (Tls::Required | Tls::RequiredInsecure, AdvTls::No, _) => {
                anyhow::bail!("tls policy is {enable_tls:?} but STARTTLS is not advertised by {address:?}:{port}");
            }
            (Tls::Disabled, _, _) => {
                // Do not use TLS
                false
            }
            (Tls::Opportunistic | Tls::OpportunisticInsecure, AdvTls::Yes, BrokenTls::Yes) => {
                // TLS is broken, do not use it
                false
            }
            (Tls::Opportunistic | Tls::OpportunisticInsecure, AdvTls::No, _) => {
                // TLS is not advertised, don't try to use it
                false
            }
            (Tls::OpportunisticInsecure, AdvTls::Yes, BrokenTls::No) => {
                let (enabled, label) = match client
                    .starttls(TlsOptions {
                        insecure: enable_tls.allow_insecure(),
                        prefer_openssl,
                        alt_name: None,
                        dane_tlsa,
                        openssl_options,
                        openssl_cipher_list,
                        openssl_cipher_suites,
                        rustls_cipher_suites,
                    })
                    .await?
                {
                    TlsStatus::FailedHandshake(handshake_error) => {
                        tracing::debug!(
                            "TLS handshake with {address}:{port} failed: \
                        {handshake_error}, but continuing in clear text because \
                        we are in OpportunisticInsecure mode"
                        );

                        self.remember_broken_tls(&dispatcher.name, &path_config);

                        if path_config.opportunistic_tls_reconnect_on_failed_handshake {
                            self.addresses.push(address);
                            return Box::pin(
                                self.attempt_connection_impl(dispatcher, Some(Tls::Disabled)),
                            )
                            .await;
                        }

                        // We did not enable TLS
                        (false, format!("failed: {handshake_error}"))
                    }
                    TlsStatus::Info(info) => {
                        // TLS is available
                        tracing::trace!("TLS: {info:?}");
                        self.tls_info.replace(info);
                        (true, "OK".to_string())
                    }
                };
                // Re-EHLO even if we didn't enable TLS, as some implementations
                // incorrectly roll over failed TLS into the following command,
                // and we want to consider those as connection errors rather than
                // having them show up per-message in MAIL FROM
                match client.ehlo_lhlo(&ehlo_name, path_config.use_lmtp).await {
                    Ok(_) => enabled,
                    Err(error) => {
                        self.remember_broken_tls(&dispatcher.name, &path_config);
                        if path_config.opportunistic_tls_reconnect_on_failed_handshake {
                            self.addresses.push(address);
                            return Box::pin(
                                self.attempt_connection_impl(dispatcher, Some(Tls::Disabled)),
                            )
                            .await;
                        }

                        return Err(OpportunisticInsecureTlsHandshakeError {
                            error,
                            address: format!("{address}:{port}"),
                            label,
                        }
                        .into());
                    }
                }
            }
            (
                Tls::Required | Tls::RequiredInsecure,
                AdvTls::Yes,
                _, /* don't care if we think tls is broken when policy is required */
            )
            | (Tls::Opportunistic, AdvTls::Yes, BrokenTls::No) => {
                match client
                    .starttls(TlsOptions {
                        insecure: enable_tls.allow_insecure(),
                        prefer_openssl,
                        alt_name: None,
                        dane_tlsa,
                        openssl_options,
                        openssl_cipher_list,
                        openssl_cipher_suites,
                        rustls_cipher_suites,
                    })
                    .await?
                {
                    TlsStatus::FailedHandshake(handshake_error) => {
                        self.remember_broken_tls(&dispatcher.name, &path_config);

                        // Don't try too hard to send the quit here; the connection may
                        // be busted by the failed handshake and never succeed
                        tokio::time::timeout(
                            tokio::time::Duration::from_secs(2),
                            client.send_command(&rfc5321::Command::Quit),
                        )
                        .await
                        .ok();

                        if path_config.opportunistic_tls_reconnect_on_failed_handshake {
                            self.addresses.push(address);
                            return Box::pin(
                                self.attempt_connection_impl(dispatcher, Some(Tls::Disabled)),
                            )
                            .await;
                        }
                        anyhow::bail!(
                            "TLS handshake with {address:?}:{port} failed: {handshake_error}"
                        );
                    }
                    TlsStatus::Info(info) => {
                        self.tracer
                            .diagnostic(Level::INFO, || format!("TLS: {info:?}"));
                        tracing::trace!("TLS: {info:?}");
                        self.tls_info.replace(info);
                    }
                }

                match client
                    .ehlo_lhlo(&ehlo_name, path_config.use_lmtp)
                    .await
                    .with_context(|| format!("{address:?}:{port}: {helo_verb} after STARTTLS"))
                {
                    Ok(_) => true,
                    Err(err) => {
                        self.remember_broken_tls(&dispatcher.name, &path_config);
                        if path_config.opportunistic_tls_reconnect_on_failed_handshake {
                            self.addresses.push(address);
                            return Box::pin(
                                self.attempt_connection_impl(dispatcher, Some(Tls::Disabled)),
                            )
                            .await;
                        }
                        return Err(err);
                    }
                }
            }
        };

        if let Some(username) = &path_config.smtp_auth_plain_username {
            if !tls_enabled && !path_config.allow_smtp_auth_plain_without_tls {
                anyhow::bail!(
                    "TLS is not enabled and AUTH PLAIN is required. Skipping ({address:?}:{port})"
                );
            }

            let password = if let Some(pw) = &path_config.smtp_auth_plain_password {
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
                .with_context(|| {
                    format!(
                        "authenticating as {username} via SMTP AUTH PLAIN to {address:?}:{port}"
                    )
                })?;
        }

        self.client
            .replace(connection_wrapper.map_connection(client));
        self.client_address.replace(address);
        dispatcher.delivered_this_connection = 0;
        Ok(())
    }

    fn remember_broken_tls(&self, site_name: &str, path_config: &EgressPathConfig) {
        if let Some(duration) = path_config.remember_broken_tls {
            BROKEN_TLS_BY_SITE.insert(
                site_name.to_string(),
                (),
                std::time::Instant::now() + duration,
            );
        }
    }

    fn has_broken_tls(&self, site_name: &str, path_config: &EgressPathConfig) -> bool {
        if path_config.remember_broken_tls.is_some() {
            BROKEN_TLS_BY_SITE.get(site_name).is_some()
        } else {
            false
        }
    }

    async fn log_disposition(
        &self,
        dispatcher: &Dispatcher,
        kind: RecordType,
        msg: Message,
        response: Response,
    ) {
        log_disposition(LogDisposition {
            kind,
            msg,
            response,
            site: &dispatcher.name,
            peer_address: self.client_address.as_ref(),
            egress_pool: Some(&dispatcher.egress_pool),
            egress_source: Some(&dispatcher.egress_source.name),
            relay_disposition: None,
            delivery_protocol: Some(&dispatcher.delivery_protocol),
            tls_info: self.tls_info.as_ref(),
            source_address: self.source_address.clone(),
            provider: dispatcher.path_config.borrow().provider_name.as_deref(),
            session_id: Some(dispatcher.session_id),
        })
        .await
    }
}

#[async_trait]
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
        self.attempt_connection_impl(dispatcher, None)
            .await
            .map_err(|err| {
                self.tracer.diagnostic(Level::ERROR, || format!("{err:#}"));
                err
            })
    }

    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        !self.addresses.is_empty()
    }

    async fn deliver_message(
        &mut self,
        mut msgs: Vec<Message>,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            msgs.len() == 1,
            "smtp_dispatcher only supports a batch size of 1"
        );
        let msg = msgs.pop().expect("just verified that there is one");

        msg.load_meta_if_needed().await.context("loading meta")?;
        msg.load_data_if_needed().await.context("loading data")?;

        let data = msg.get_data();
        let sender: ReversePath = msg
            .sender()?
            .try_into()
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let recipient: ForwardPath = msg
            .recipient()?
            .try_into()
            .map_err(|err| anyhow::anyhow!("{err}"))?;

        self.tracer.set_meta("sender", sender.to_string());
        self.tracer.set_meta("recipient", recipient.to_string());
        if let Ok(name) = msg.get_queue_name() {
            let components = QueueNameComponents::parse(&name);
            self.tracer.set_meta("domain", components.domain);
            self.tracer
                .set_meta("routing_domain", components.routing_domain);
            match &components.campaign {
                Some(campaign) => {
                    self.tracer.set_meta("campaign", campaign.to_string());
                }
                None => {
                    self.tracer.unset_meta("campaign");
                }
            }
            match &components.tenant {
                Some(tenant) => {
                    self.tracer.set_meta("tenant", tenant.to_string());
                }
                None => {
                    self.tracer.unset_meta("tenant");
                }
            }
            self.tracer.set_meta("queue_name", name);
        }
        self.tracer
            .submit(|| SmtpClientTraceEventPayload::MessageObtained);

        match self
            .client
            .as_mut()
            .unwrap()
            .send_mail(sender, recipient, &*data)
            .await
        {
            Err(ClientError::Rejected(mut response)) => {
                let queue_name = msg.get_queue_name()?;
                let components = QueueNameComponents::parse(&queue_name);
                let mut config = load_config().await.context("load_config")?;

                let sig = CallbackSignature::<
                    (String, &str, Option<&str>, Option<&str>, &str),
                    Option<u16>,
                >::new("smtp_client_rewrite_delivery_status");

                let rewritten_code: anyhow::Result<Option<u16>> = config
                    .async_call_callback(
                        &sig,
                        (
                            response.to_single_line(),
                            components.domain,
                            components.tenant,
                            components.campaign,
                            components
                                .routing_domain
                                .as_deref()
                                .unwrap_or(&components.domain),
                        ),
                    )
                    .await;

                match rewritten_code {
                    Ok(Some(code)) if code != response.code => {
                        response.content = format!(
                            "{} (kumomta: status was rewritten from {} -> {code})",
                            response.content, response.code
                        );
                        response.code = code;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::error!("smtp_client_rewrite_delivery_status event failed: {err:#}. Preserving original DSN");
                    }
                }

                if response.code == 503 || (response.code >= 300 && response.code < 400) {
                    // 503 is a "permanent" failure response but it indicates
                    // that there was a protocol synchronization issue.
                    //
                    // For 3xx: there isn't a valid RFC-defined 300 final
                    // disposition for submitting an email message.  In order
                    // to get here there has most likely been a protocol
                    // synchronization issue.
                    //
                    // We should consider the connection to be broken and we
                    // should allow the message to be retried later on.
                    tracing::error!(
                        "Unexpected {} response while sending message \
                        to {} {:?}: {response:?}. \
                        Probable protocol synchronization error, please report this! \
                        Session ID={}. \
                        Message will be re-queued.",
                        response.code,
                        dispatcher.name,
                        self.client_address,
                        dispatcher.session_id,
                    );
                    if let Some(msg) = dispatcher.msgs.pop() {
                        self.log_disposition(
                            dispatcher,
                            RecordType::TransientFailure,
                            msg.clone(),
                            response.clone(),
                        )
                        .await;
                        spawn(
                            "requeue message".to_string(),
                            QueueManager::requeue_message(
                                msg,
                                IncrementAttempts::Yes,
                                None,
                                response,
                            ),
                        )?;
                    }
                    dispatcher.metrics.inc_transfail();
                    // Break this connection
                    anyhow::bail!("Protocol synchronization error!");
                } else if response.code >= 400 && response.code < 500 {
                    // Transient failure
                    tracing::debug!(
                        "failed to send message to {} {:?}: {response:?}",
                        dispatcher.name,
                        self.client_address
                    );
                    if let Some(msg) = dispatcher.msgs.pop() {
                        self.log_disposition(
                            dispatcher,
                            RecordType::TransientFailure,
                            msg.clone(),
                            response.clone(),
                        )
                        .await;
                        spawn(
                            "requeue message".to_string(),
                            QueueManager::requeue_message(
                                msg,
                                IncrementAttempts::Yes,
                                None,
                                response,
                            ),
                        )?;
                    }
                    dispatcher.metrics.inc_transfail();
                } else if response.code >= 200 && response.code < 300 {
                    tracing::debug!("Delivered OK! {response:?}");
                    if let Some(msg) = dispatcher.msgs.pop() {
                        self.log_disposition(
                            dispatcher,
                            RecordType::Delivery,
                            msg.clone(),
                            response,
                        )
                        .await;
                        SpoolManager::remove_from_spool(*msg.id()).await?;
                    }
                    dispatcher.metrics.inc_delivered();
                } else {
                    dispatcher.metrics.inc_fail();
                    tracing::debug!(
                        "failed to send message to {} {:?}: {response:?}",
                        dispatcher.name,
                        self.client_address
                    );
                    if let Some(msg) = dispatcher.msgs.pop() {
                        self.log_disposition(dispatcher, RecordType::Bounce, msg.clone(), response)
                            .await;
                        SpoolManager::remove_from_spool(*msg.id()).await?;
                    }
                }
            }
            Err(ClientError::TimeOutRequest { command, duration }) => {
                // Transient failure
                let reason = format!(
                    "failed to send message to {} {:?}: \
                    Timed Out waiting {duration:?} to write {command:?}",
                    dispatcher.name, self.client_address
                );
                tracing::debug!("{reason}");
                if let Some(msg) = dispatcher.msgs.pop() {
                    let response = Response {
                        code: 421,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 2,
                        }),
                        content: reason.clone(),
                        command: Some(command.encode()),
                    };
                    self.log_disposition(
                        dispatcher,
                        RecordType::TransientFailure,
                        msg.clone(),
                        response.clone(),
                    )
                    .await;
                    spawn(
                        "requeue message".to_string(),
                        QueueManager::requeue_message(msg, IncrementAttempts::Yes, None, response),
                    )?;
                }
                dispatcher.metrics.inc_transfail();
                // Move on to the next host
                anyhow::bail!("{reason}");
            }
            Err(ClientError::TimeOutResponse { command, duration }) => {
                // Transient failure
                let reason = format!(
                    "failed to send message to {} {:?}: \
                    Timed Out waiting {duration:?} for response to {command:?}",
                    dispatcher.name, self.client_address
                );

                tracing::debug!("{reason}");
                if let Some(msg) = dispatcher.msgs.pop() {
                    let response = Response {
                        code: 421,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 2,
                        }),
                        content: reason.clone(),
                        command: command.map(|c| c.encode()),
                    };
                    self.log_disposition(
                        dispatcher,
                        RecordType::TransientFailure,
                        msg.clone(),
                        response.clone(),
                    )
                    .await;
                    spawn(
                        "requeue message".to_string(),
                        QueueManager::requeue_message(msg, IncrementAttempts::Yes, None, response),
                    )?;
                }
                dispatcher.metrics.inc_transfail();
                // Move on to the next host
                anyhow::bail!("{reason}");
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
                if let Some(msg) = dispatcher.msgs.pop() {
                    self.log_disposition(dispatcher, RecordType::Delivery, msg.clone(), response)
                        .await;
                    SpoolManager::remove_from_spool(*msg.id()).await?;
                }
                dispatcher.metrics.inc_delivered();
            }
        };

        let is_connected = self
            .client
            .as_ref()
            .map(|c| c.is_connected())
            .unwrap_or(false);
        if !is_connected {
            anyhow::bail!(
                "after previous send attempt, client is unexpectedly no longer connected"
            );
        }

        Ok(())
    }
}
