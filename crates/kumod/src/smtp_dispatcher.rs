use crate::delivery_metrics::MetricsWrappedConnection;
use crate::http_server::admin_trace_smtp_client_v1::{
    SmtpClientTraceEventPayload, SmtpClientTracerImpl,
};
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::{IncrementAttempts, InsertReason, QueueManager, QueueState};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::smtp_server::ShuttingDownError;
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use bounce_classify::{BounceClass, PreDefinedBounceClass};
use config::{load_config, CallbackSignature};
use data_loader::KeySource;
use dns_resolver::{has_colon_port, resolve_a_or_aaaa, ResolvedMxAddresses};
use kumo_address::socket::SocketAddress;
use kumo_api_types::egress_path::{EgressPathConfig, ReconnectStrategy, Tls};
use kumo_log_types::{MaybeProxiedSourceAddress, ResolvedAddress};
use kumo_server_lifecycle::ShutdownSubcription;
use kumo_server_runtime::spawn;
use message::message::QueueNameComponents;
use message::Message;
use mta_sts::policy::PolicyMode;
use rfc5321::{
    ClientError, EnhancedStatusCode, ForwardPath, IsTooManyRecipients, Response, ReversePath,
    SmtpClient, TlsInformation, TlsOptions, TlsStatus,
};
use serde::{Deserialize, Serialize};
use spool::SpoolId;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UnixStream;
use tracing::Level;

lruttl::declare_cache! {
/// Remembers which site names have broken TLS
static BROKEN_TLS_BY_SITE: LruCacheWithTtl<String, ()>::new("smtp_dispatcher_broken_tls", 64 * 1024);
}

lruttl::declare_cache! {
/// Caches smtp client certificates by KeySource spec
static CLIENT_CERT: LruCacheWithTtl<KeySource, Result<Option<Arc<Box<[u8]>>>, String>>::new("smtp_dispatcher_client_certificate", 1024);
}

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

impl MxListEntry {
    /// Resolve self into 1 or more `ResolvedAddress` and append to the
    /// supplied `addresses` vector.
    pub async fn resolve_into(&self, addresses: &mut Vec<ResolvedAddress>) -> anyhow::Result<()> {
        match self {
            Self::Name(a) => {
                if let Some((label, port)) = has_colon_port(a) {
                    let resolved = resolve_a_or_aaaa(label, None)
                        .await
                        .with_context(|| format!("resolving mx_list entry {a}"))?;
                    for mut r in resolved {
                        r.addr.set_port(port);
                        addresses.push(r);
                    }

                    return Ok(());
                }

                addresses.append(
                    &mut resolve_a_or_aaaa(a, None)
                        .await
                        .with_context(|| format!("resolving mx_list entry {a}"))?,
                );
            }
            Self::Resolved(addr) => {
                addresses.push(addr.clone());
            }
        };
        Ok(())
    }

    /// Return a label that will be used as part of the synthesized site_name
    /// for a manually provided mx_list style site, rather than the more
    /// typical resolved-via-MX-records site name.
    /// We return the stringy versions of those manually specified addresses
    pub fn label(&self) -> String {
        match self {
            Self::Name(a) => a.to_string(),
            Self::Resolved(addr) => addr.addr.to_string(),
        }
    }
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
    site_has_broken_tls: bool,
    terminated_ok: bool,
    attempted_message_send: bool,
    recips_last_txn: HashMap<(SpoolId, ForwardPath), u8>,
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
                a.resolve_into(&mut addresses).await?;
            }
            // Note that ResolvedMxAddresses::Addresses is in LIFO
            // order, and we have FIFO order.  Reverse it!
            addresses.reverse();
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
                    .bulk_ready_queue_operation(
                        Response {
                            code: 556,
                            enhanced_code: Some(EnhancedStatusCode {
                                class: 5,
                                subject: 1,
                                detail: 10,
                            }),
                            content: "Recipient address has a null MX".to_string(),
                            command: None,
                        },
                        InsertReason::FailedDueToNullMx.into(),
                    )
                    .await;
                return Ok(None);
            }
            ResolvedMxAddresses::Addresses(a) => a,
        };

        if addresses.is_empty() {
            dispatcher
                .bulk_ready_queue_operation(
                    Response {
                        code: 451,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 4,
                        }),
                        content: "MX didn't resolve to any hosts".to_string(),
                        command: None,
                    },
                    InsertReason::MxResolvedToZeroHosts.into(),
                )
                .await;
            return Ok(None);
        }

        for addr in &addresses {
            if let Some(ip) = addr.addr.ip() {
                if path_config.prohibited_hosts.contains(ip) {
                    dispatcher
                        .bulk_ready_queue_operation(
                            Response {
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
                            },
                            InsertReason::MxWasProhibited.into(),
                        )
                        .await;
                    return Ok(None);
                }
            }
        }

        addresses.retain(|addr| match addr.addr.ip() {
            Some(ip) => !path_config.skip_hosts.contains(ip),
            None => true,
        });

        if addresses.is_empty() {
            dispatcher
                .bulk_ready_queue_operation(
                    Response {
                        code: 550,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 5,
                            subject: 4,
                            detail: 4,
                        }),
                        content: "MX consisted solely of hosts on the skip_hosts list".to_string(),
                        command: None,
                    },
                    InsertReason::MxWasSkipped.into(),
                )
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
            site_has_broken_tls: false,
            terminated_ok: false,
            attempted_message_send: false,
            recips_last_txn: HashMap::new(),
        }))
    }

    async fn attempt_connection_impl(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        if let Some(client) = &self.client {
            if client.is_connected() {
                return Ok(());
            }
        }

        let mut shutdown = ShutdownSubcription::get();

        let path_config = dispatcher.path_config.borrow();
        if let Some(throttle) = path_config.max_connection_rate {
            loop {
                let result = throttle
                    .throttle(format!("{}-connection-rate", dispatcher.name))
                    .await?;

                if let Some(delay) = result.retry_after {
                    dispatcher
                        .states
                        .lock()
                        .connection_rate_throttled
                        .replace(QueueState::new(format!(
                            "max_connection_rate throttling for {delay:?}"
                        )));

                    if delay >= path_config.client_timeouts.idle_timeout {
                        self.tracer.diagnostic(Level::INFO, || {
                            format!(
                                "Connection rate throttled for {delay:?} which is \
                                 longer than the idle_timeout, will disconnect"
                            )
                        });
                        dispatcher
                            .throttle_ready_queue(
                                delay,
                                InsertReason::ConnectionRateThrottle.into(),
                            )
                            .await;
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
                            return Err(ShuttingDownError.into());
                        }
                    };
                } else {
                    break;
                }
            }
            dispatcher.states.lock().connection_rate_throttled.take();
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
        let mut enable_tls = path_config.enable_tls;
        let port = dispatcher
            .egress_source
            .remote_port
            .or_else(|| address.addr.port())
            .unwrap_or(path_config.smtp_port);

        let target_address: SocketAddress = match address.addr.ip() {
            Some(ip) => SocketAddr::new(ip, port).into(),
            None => {
                let unix = address.addr.unix().expect("ip case handled above");
                unix.into()
            }
        };

        let connect_context = format!("connect to {target_address} and read initial banner");

        self.tracer.diagnostic(Level::INFO, || {
            format!("Attempting connection to {target_address}")
        });

        let make_connection = {
            let target_address = target_address.clone();
            let address = address.clone();
            let timeouts = path_config.client_timeouts;
            let egress_source = dispatcher.egress_source.clone();
            let tracer = self.tracer.clone();
            let enable_rset = path_config.enable_rset;
            let enable_pipelining = path_config.enable_pipelining;

            // We need to spawn the connection attempt into another task,
            // otherwise the select! invocation below won't run it in parallel with
            // awaiting the shutdown subscription, causing us to uselessly wait
            // for the full connect timeout during shutdown.
            tokio::spawn(async move {
                let (mut client, source_address) = match address.addr.ip() {
                    Some(ip) => {
                        let (stream, source_address) = egress_source
                            .connect_to(SocketAddr::new(ip, port), timeouts.connect_timeout)
                            .await?;
                        tracing::debug!(
                            "connected to {target_address} via source address {source_address:?}"
                        );

                        let client = SmtpClient::with_stream(stream, &mx_host, timeouts);
                        (client, source_address)
                    }
                    None => {
                        let unix = address.addr.unix().expect("ip case handled above");
                        let path = unix.as_pathname().ok_or_else(|| {
                            anyhow::anyhow!(
                                "cannot connect to an unbound unix domain socket address"
                            )
                        })?;

                        let stream = UnixStream::connect(&path).await?;

                        let source_address = MaybeProxiedSourceAddress {
                            address: stream.local_addr()?.into(),
                            server: None,
                            protocol: None,
                        };

                        tracing::debug!(
                            "connected to {target_address} via source address {source_address:?}"
                        );

                        let client = SmtpClient::with_stream(stream, &mx_host, timeouts);
                        (client, source_address)
                    }
                };

                tracer.set_meta("source_address", source_address.address.to_string());
                tracer.set_meta("mx_host", mx_host.to_string());
                tracer.set_meta("mx_address", address.addr.to_string());
                tracer.submit(|| SmtpClientTraceEventPayload::Connected);

                client.set_tracer(tracer);
                client.set_enable_pipelining(enable_pipelining);
                client.set_enable_rset(enable_rset);

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
                return Err(ShuttingDownError.into());
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
            .with_context(|| format!("{target_address}: {helo_verb} after banner"))?;

        // Use STARTTLS if available.
        let has_tls = pretls_caps.contains_key("STARTTLS");
        let broken_tls = self.has_broken_tls(&dispatcher.name);

        let mut dane_tlsa = vec![];
        let mut mta_sts_eligible = true;

        let mut certificate_from_pem = None;
        let mut private_key_from_pem = None;

        if let Some(pem) = &path_config.tls_certificate {
            certificate_from_pem = self.resolve_cached_client_cert(pem).await?
        }

        if let Some(pem) = &path_config.tls_private_key {
            private_key_from_pem = self.resolve_cached_client_cert(pem).await?
        }

        let openssl_options = path_config.openssl_options;
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
                        certificate_from_pem,
                        private_key_from_pem,
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

                        self.remember_broken_tls(&dispatcher.name, &path_config)
                            .await;

                        if path_config.opportunistic_tls_reconnect_on_failed_handshake {
                            self.addresses.push(address);
                            anyhow::bail!(
                                "TLS handshake failed: {handshake_error}, \
                                will re-connect in the clear because \
                                opportunistic_tls_reconnect_on_failed_handshake=true"
                            );
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
                        self.remember_broken_tls(&dispatcher.name, &path_config)
                            .await;
                        if path_config.opportunistic_tls_reconnect_on_failed_handshake {
                            self.addresses.push(address);
                            anyhow::bail!(
                                "{helo_verb} after STARTLS failed: {error:#}, \
                                will re-connect in the clear because \
                                opportunistic_tls_reconnect_on_failed_handshake=true"
                            );
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
                        certificate_from_pem,
                        private_key_from_pem,
                        openssl_options,
                        openssl_cipher_list,
                        openssl_cipher_suites,
                        rustls_cipher_suites,
                    })
                    .await?
                {
                    TlsStatus::FailedHandshake(handshake_error) => {
                        self.remember_broken_tls(&dispatcher.name, &path_config)
                            .await;

                        // Don't try too hard to send the quit here; the connection may
                        // be busted by the failed handshake and never succeed
                        tokio::time::timeout(
                            tokio::time::Duration::from_secs(2),
                            client.send_command(&rfc5321::Command::Quit),
                        )
                        .await
                        .ok();

                        if enable_tls.is_opportunistic()
                            && path_config.opportunistic_tls_reconnect_on_failed_handshake
                        {
                            self.addresses.push(address);
                            anyhow::bail!(
                                "TLS handshake failed: {handshake_error}, will \
                                re-connect in the clear because \
                                opportunistic_tls_reconnect_on_failed_handshake=true"
                            );
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
                        self.remember_broken_tls(&dispatcher.name, &path_config)
                            .await;
                        if enable_tls.is_opportunistic()
                            && path_config.opportunistic_tls_reconnect_on_failed_handshake
                        {
                            self.addresses.push(address);
                            anyhow::bail!(
                                "{helo_verb} after STARTLS failed {err:#}, \
                                will re-connect in the clear because \
                                opportunistic_tls_reconnect_on_failed_handshake=true"
                            );
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

    async fn resolve_cached_client_cert(
        &mut self,
        source: &KeySource,
    ) -> anyhow::Result<Option<Arc<Box<[u8]>>>> {
        CLIENT_CERT
            .get_or_try_insert(source, |_| tokio::time::Duration::from_secs(300), async {
                let data = source
                    .get()
                    .await
                    .map(|vec| Some(Arc::new(vec.into_boxed_slice())))
                    .map_err(|e| e.to_string());
                Ok::<_, anyhow::Error>(data)
            })
            .await
            .map_err(|e| anyhow::Error::msg(e.to_string()))?
            .item
            .map_err(|e| anyhow::Error::msg(e.to_string()))
    }

    async fn remember_broken_tls(&mut self, site_name: &str, path_config: &EgressPathConfig) {
        let duration = match path_config.remember_broken_tls {
            Some(duration) => Some(duration),
            None if path_config.opportunistic_tls_reconnect_on_failed_handshake => {
                Some(std::time::Duration::from_secs(15 * 60))
            }
            None => None,
        };
        if let Some(duration) = duration {
            self.site_has_broken_tls = true;
            BROKEN_TLS_BY_SITE
                .insert(
                    site_name.to_string(),
                    (),
                    tokio::time::Instant::now() + duration,
                )
                .await;
        }
    }

    fn has_broken_tls(&self, site_name: &str) -> bool {
        self.site_has_broken_tls || BROKEN_TLS_BY_SITE.get(site_name).is_some()
    }

    async fn log_disposition(
        &self,
        dispatcher: &Dispatcher,
        kind: RecordType,
        msg: Message,
        recipient_list: Option<Vec<String>>,
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
            recipient_list: recipient_list,
        })
        .await
    }

    /// Prepare for a potential reconnect.
    /// Returns true if there are potential addresses that we could
    /// reconnect to
    fn update_state_for_reconnect(&mut self, dispatcher: &mut Dispatcher) -> bool {
        match dispatcher.path_config.borrow().reconnect_strategy {
            ReconnectStrategy::TerminateSession => {
                self.addresses.clear();
            }
            ReconnectStrategy::ReconnectSameHost => {
                if let Some(address) = self.client_address.take() {
                    self.addresses.push(address);
                }
            }
            ReconnectStrategy::ConnectNextHost => {
                // Nothing needed; we're naturally set up to do this
            }
        }
        self.client.take();

        // We consider our session to be terminated with "success" if
        // we got as far as trying to send a message down any of them.
        // The "success" part really is just short-cutting and preventing
        // the "no more hosts" error that we would otherwise generate,
        // which can "stun" the next message in the ready queue in a
        // situation where we actually know enough about the situation
        // that we should try a new session.
        if self.addresses.is_empty() {
            self.terminated_ok = self.attempted_message_send;
        }

        !self.addresses.is_empty()
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

    /// See the logic in update_state_for_reconnect for when we consider
    /// our session to be terminated
    async fn has_terminated_ok(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        self.terminated_ok
    }

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        self.attempt_connection_impl(dispatcher)
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
        let data = msg.data().await.context("loading data")?;

        let spool_id = *msg.id();
        let mut recips_this_txn = HashMap::new();

        let sender: ReversePath = msg
            .sender()?
            .try_into()
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let mut recipients: Vec<ForwardPath> = vec![];
        for recip in msg.recipient_list()? {
            let recip: ForwardPath = recip.try_into().map_err(|err| anyhow::anyhow!("{err:#}"))?;
            recips_this_txn.insert(
                (spool_id, recip.clone()),
                1 + self
                    .recips_last_txn
                    .get(&(spool_id, recip.clone()))
                    .copied()
                    .unwrap_or(0),
            );
            recipients.push(recip);
        }

        self.tracer.set_meta("message_id", msg.id().to_string());
        self.tracer.set_meta("sender", sender.to_string());
        self.tracer
            .set_meta("recipient", msg.recipient_list_string()?);
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

        self.attempted_message_send = true;
        let try_next_host_on_transport_error = dispatcher
            .path_config
            .borrow()
            .try_next_host_on_transport_error;

        self.client.as_mut().map(|client| {
            client.set_ignore_8bit_checks(dispatcher.path_config.borrow().ignore_8bit_checks)
        });

        // This will hold the list of recipients that have not
        // reached a terminal disposition
        let mut revised_recipient_list = vec![];
        let mut recipients_this_batch = vec![];
        let mut retry_immediately = false;

        let path_config = dispatcher.path_config.borrow();
        for recip in recipients {
            if recipients_this_batch.len() < path_config.max_recipients_per_batch {
                recipients_this_batch.push(recip);
            } else {
                revised_recipient_list.push(recip.into());
                // The excess is ready to go immediately
                retry_immediately = true;
            }
        }

        let send_result = self
            .client
            .as_mut()
            .unwrap()
            .send_mail_multi_recip(sender, recipients_this_batch.clone(), &*data)
            .await;

        let mut result_per_rcpt = vec![];
        let mut rewrite_eligible = false;
        let mut break_connection = false;
        let mut overall_response = None;

        match send_result {
            Err(ClientError::RejectedBatch(responses)) => {
                rewrite_eligible = true;
                for resp in responses {
                    result_per_rcpt.push(resp.clone());
                }
            }
            Err(ClientError::Rejected(response)) => {
                rewrite_eligible = true;
                for _recip in &recipients_this_batch {
                    result_per_rcpt.push(response.clone());
                }
            }
            Err(
                ref err @ ClientError::TimeOutRequest {
                    ref commands,
                    duration,
                },
            ) => {
                break_connection = true;
                let reason = format!(
                    "KumoMTA internal: failed to send message to {} {:?}: \
                    Timed Out waiting {duration:?} to write {commands:?}",
                    dispatcher.name, self.client_address
                );
                tracing::debug!("{reason}");

                for _recip in &recipients_this_batch {
                    result_per_rcpt.push(Response {
                        code: 421,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 2,
                        }),
                        content: reason.clone(),
                        command: err.command(),
                    });
                }
            }
            Err(ClientError::TimeOutResponse { command, duration }) => {
                break_connection = true;
                let reason = format!(
                    "KumoMTA internal: failed to send message to {} {:?}: \
                    Timed Out waiting {duration:?} for response to {command:?}",
                    dispatcher.name, self.client_address
                );

                for _recip in &recipients_this_batch {
                    result_per_rcpt.push(Response {
                        code: 421,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 2,
                        }),
                        content: reason.clone(),
                        command: command.as_ref().map(|c| c.encode()),
                    });
                }
            }
            Err(err) => {
                for _recip in &recipients_this_batch {
                    result_per_rcpt.push(Response {
                        code: 400,
                        enhanced_code: None,
                        content: format!("KumoMTA internal: failed to send message: {err:#}"),
                        command: err.command(),
                    });
                }
            }
            Ok(status) => {
                for resp in &status.rcpt_responses {
                    if resp.code == 250 {
                        result_per_rcpt.push(status.response.clone());
                    } else {
                        rewrite_eligible = true;
                        result_per_rcpt.push(resp.clone());
                    }
                }
                overall_response.replace(status.response);
            }
        }

        if rewrite_eligible {
            let queue_name = msg.get_queue_name()?;
            let components = QueueNameComponents::parse(&queue_name);

            let sig = CallbackSignature::<
                (String, &str, Option<&str>, Option<&str>, &str),
                Option<u16>,
            >::new("smtp_client_rewrite_delivery_status");

            for response in result_per_rcpt.iter_mut() {
                if response.code == 250 {
                    continue;
                }
                let mut config = load_config().await.context("load_config")?;
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
                if rewritten_code.is_ok() {
                    config.put();
                }

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
            }
        }

        struct ByStatusEntry {
            record_type: RecordType,
            recipients: Vec<ForwardPath>,
        }

        // Group by distinct response for logging purposes
        let mut by_status = HashMap::<(Response, IsTooManyRecipients), ByStatusEntry>::new();
        let mut by_class = HashMap::new();

        let mut transport_error = false;

        for (batch_idx, (recipient, response)) in recipients_this_batch
            .iter()
            .zip(result_per_rcpt.iter())
            .enumerate()
        {
            let (record_type, too_many) = classify_record(&response).await;

            // Determine effective too-many recip state: if the batch is size 1,
            // or this is the first recipient, it cannot be too-many
            let too_many = if batch_idx == 0 || recipients_this_batch.len() == 1 {
                IsTooManyRecipients::No
            } else {
                too_many
            };
            let too_many = if too_many == IsTooManyRecipients::Maybe
                && self
                    .recips_last_txn
                    .get(&(spool_id, recipient.clone()))
                    .copied()
                    .unwrap_or(0)
                    < 1
            {
                IsTooManyRecipients::Yes
            } else if too_many == IsTooManyRecipients::Yes {
                IsTooManyRecipients::Yes
            } else {
                IsTooManyRecipients::No
            };

            if record_type == RecordType::TransientFailure {
                revised_recipient_list.push(recipient.clone().into());
            }
            if record_type != RecordType::Delivery && overall_response.is_none() {
                overall_response.replace(response.clone());
            }
            match record_type {
                RecordType::TransientFailure => {
                    dispatcher.metrics.inc_transfail();
                }
                RecordType::Delivery => {
                    dispatcher.metrics.inc_delivered();
                }
                RecordType::Bounce => {
                    dispatcher.metrics.inc_fail();
                }
                _ => unreachable!(),
            }

            if too_many == IsTooManyRecipients::Yes {
                retry_immediately = true;
            } else if !response.was_due_to_message() {
                transport_error = true;
            }
            if response.code == 421 {
                break_connection = true;
            }

            by_status
                .entry((response.clone(), too_many))
                .or_insert_with(|| ByStatusEntry {
                    record_type,
                    recipients: vec![],
                })
                .recipients
                .push(recipient.clone());
            *by_class.entry(record_type).or_insert(0) += 1;
        }

        self.recips_last_txn = recips_this_txn;

        let mut logged_transient = false;

        // Log the various outcomes
        for ((response, too_many), entry) in by_status {
            if entry.record_type == RecordType::TransientFailure {
                if too_many == IsTooManyRecipients::Yes
                //    && by_class[&RecordType::TransientFailure] == 1
                //    && by_class.len() > 1
                {
                    // Skip logging a transient failure for the too many
                    // recipients case if it looks like something got through
                    // OK.  We'll retry the excess recipients immediately
                    // and have a log record for those imminently.
                    continue;
                }
                logged_transient = true;
            }

            self.log_disposition(
                dispatcher,
                entry.record_type,
                msg.clone(),
                Some(
                    entry
                        .recipients
                        .into_iter()
                        .map(|fp| fp.to_string())
                        .collect(),
                ),
                response,
            )
            .await;
        }

        if revised_recipient_list.is_empty() {
            // No more recipients means that we can stop tracking
            // this message and remove it from the spool
            dispatcher.msgs.pop();
            SpoolManager::remove_from_spool(*msg.id()).await?;

            let is_connected = self
                .client
                .as_ref()
                .map(|c| c.is_connected())
                .unwrap_or(false);
            if !is_connected {
                self.update_state_for_reconnect(dispatcher);
                anyhow::bail!(
                    "after previous send attempt, client is unexpectedly no longer connected"
                );
            }

            Ok(())
        } else {
            // Revise the recipient list; all delivered and bounced
            // recipients are removed leaving just those that need
            // the message to be re-attempted
            msg.set_recipient_list(revised_recipient_list)?;
            dispatcher.msgs.pop();

            if transport_error && try_next_host_on_transport_error {
                break_connection = true;
            }

            if break_connection && try_next_host_on_transport_error {
                let have_more_connection_candidates = self.update_state_for_reconnect(dispatcher);
                if have_more_connection_candidates {
                    // Try it on the next connection
                    dispatcher.msgs.push(msg);
                    return Ok(());
                }
            }

            let is_connected = self
                .client
                .as_ref()
                .map(|c| c.is_connected())
                .unwrap_or(false);

            if retry_immediately && is_connected {
                dispatcher.msgs.push(msg);
                return Ok(());
            }

            spawn(
                "requeue message",
                QueueManager::requeue_message(
                    msg,
                    IncrementAttempts::Yes,
                    None,
                    overall_response.take().unwrap_or_else(|| Response {
                        code: 400,
                        enhanced_code: None,
                        command: None,
                        content: "KumoMTA internal: retrying failed batch".to_string(),
                    }),
                    if logged_transient {
                        InsertReason::LoggedTransientFailure.into()
                    } else {
                        InsertReason::TooManyRecipients.into()
                    },
                ),
            )?;

            if !is_connected {
                self.update_state_for_reconnect(dispatcher);
                anyhow::bail!(
                    "after previous send attempt, client is unexpectedly no longer connected"
                );
            }
            Ok(())
        }
    }
}

async fn classify_record(response: &Response) -> (RecordType, IsTooManyRecipients) {
    let too_many = match response.is_too_many_recipients() {
        as_is @ (IsTooManyRecipients::Yes | IsTooManyRecipients::No) => as_is,
        IsTooManyRecipients::Maybe => {
            match crate::logging::classify::classify_response(&response).await {
                BounceClass::UserDefined(_) => IsTooManyRecipients::Maybe,
                BounceClass::PreDefined(bc) => match bc {
                    PreDefinedBounceClass::TooManyRecipients => IsTooManyRecipients::Yes,
                    PreDefinedBounceClass::Uncategorized => IsTooManyRecipients::Maybe,
                    _ => IsTooManyRecipients::No,
                },
            }
        }
    };

    let record_type = if matches!(
        too_many,
        IsTooManyRecipients::Yes | IsTooManyRecipients::Maybe
    ) || response.code == 503
        || (response.code >= 300 && response.code < 500)
    {
        // 503 is a "permanent" failure response but it indicates
        // that there was a protocol synchronization issue.
        //
        // For 3xx: there isn't a valid RFC-defined 300 final
        // disposition for submitting an email message.  In order
        // to get here there has most likely been a protocol
        // synchronization issue.
        RecordType::TransientFailure
    } else if response.code >= 200 && response.code < 300 {
        RecordType::Delivery
    } else {
        RecordType::Bounce
    };

    (record_type, too_many)
}
