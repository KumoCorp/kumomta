use crate::delivery_metrics::MetricsWrappedConnection;
use crate::http_server::admin_trace_smtp_client_v1::{
    SmtpClientTraceEventPayload, SmtpClientTracerImpl,
};
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::{IncrementAttempts, InsertReason, QueueManager, QueueState};
use crate::ready_queue::{AttemptConnectionDisposition, Dispatcher, QueueDispatcher};
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use bounce_classify::{BounceClass, PreDefinedBounceClass};
use config::{load_config, CallbackSignature};
use data_loader::KeySource;
use dns_resolver::{
    has_colon_port, resolve_a_or_aaaa, DaneStatus, IpLookupStrategy, SecureCnameStatus,
};
use kumo_address::socket::SocketAddress;
use kumo_api_types::egress_path::{EgressPathConfig, ReconnectStrategy, Tls};
use kumo_log_types::{MaybeProxiedSourceAddress, ResolvedAddress};
use kumo_prometheus::declare_metric;
use kumo_server_lifecycle::{ShutdownSubcription, ShuttingDownError};
use kumo_server_runtime::spawn;
use mailexchanger::{PolicyMode, ResolvedMxAddresses};
use message::message::QueueNameComponents;
use message::Message;
use rfc5321::parser::{EnvelopeAddress, ForwardPath, ReversePath};
use rfc5321::{
    ClientError, EnhancedStatusCode, IsTooManyRecipients, Response, SmtpClient, TlsInformation,
    TlsOptions, TlsStatus,
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

declare_metric! {
/// Number of DANE policy decisions made on the SMTP delivery path, labelled by
/// `result`.
///
/// {{since('dev')}}
///
/// The `result` label is one of:
///
///   * `ok`: usable DANE-TA(2)/DANE-EE(3) TLSA records were found; the peer
///     certificate is checked against them.
///   * `unusable`: TLSA records were published but none are usable; STARTTLS is
///     required but the peer certificate is not checked.
///   * `not_applicable`: the chain to the MX host was DNSSEC-validated but there
///     are no TLSA records (securely absent), so DANE does not apply.
///   * `insecure_chain`: DANE is enabled but the chain to the MX host was not
///     DNSSEC-validated, so DANE does not apply. A persistently high value here
///     with none of the other results can indicate that the resolver is not
///     performing DNSSEC validation.
///   * `tempfail`: the TLSA lookup could not be securely resolved (SERVFAIL,
///     timeout, or bogus); delivery is deferred.
///
/// These are counters; reason about them as rates.
///
/// **Confirming DANE is working:** with `enable_dane = true`, a healthy
/// deployment shows a steady stream of `not_applicable` (most DNSSEC-signed
/// domains do not publish TLSA records) plus some `ok` for the destinations
/// that do. The single most useful health check is: if you only ever see
/// `insecure_chain` and never `ok` or `not_applicable`, your
/// resolver is almost certainly not performing DNSSEC validation, so DANE is
/// silently doing nothing — verify that you configured a validating resolver.
/// For an ad-hoc check that does not require standing up a sink or large-scale
/// test, <https://havedane.net> publishes known-good TLSA records: send a test
/// message to an address there and confirm that `ok` increments.
///
/// **What to alert on:**
///
///   * A sustained or rising rate of `tempfail` is the highest-signal problem:
///     each one is a *deferred delivery* because the TLSA lookup could not be
///     securely resolved. This usually points at resolver or upstream-DNS
///     trouble (SERVFAIL, timeouts, bogus answers), and only rarely at an
///     active downgrade attempt; either way, mail is being delayed, so it is
///     worth paging on.
///   * `ok` pinned at zero while `insecure_chain` is high (with
///     `enable_dane = true`) indicates a non-validating resolver, i.e. DANE is
///     not engaging at all.
///   * `unusable` is informational: a remote operator published TLSA records
///     that are not usable for SMTP DANE (for example, only PKIX usages). A low
///     background level is normal and reflects the remote side, not your
///     infrastructure.
///   * `not_applicable` and `insecure_chain` are expected in normal operation
///     for the large fraction of destinations that do not publish usable TLSA
///     records or are not DNSSEC-signed; do not alert on these in isolation.
static DANE_RESULT: IntCounterVec(
        "dane_result_count",
        &["result"]);
}

fn record_dane_result(result: &str) {
    if let Ok(counter) = DANE_RESULT.get_metric_with_label_values(&[result]) {
        counter.inc();
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq)]
pub struct SmtpProtocol {
    #[serde(default)]
    pub mx_list: Vec<MxListEntry>,

    /// Treat the hosts in `mx_list` as a trusted (DNSSEC-secure) MX selection
    /// for the purposes of DANE downgrade resistance. Leave this false (the
    /// default) when `mx_list` is derived from an untracked DNS lookup;
    /// otherwise a spoofed lookup could let an attacker-chosen host pass DANE.
    #[serde(default)]
    pub treat_mx_list_as_secure: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
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
    pub async fn resolve_into(
        &self,
        addresses: &mut Vec<ResolvedAddress>,
        strategy: IpLookupStrategy,
    ) -> anyhow::Result<()> {
        match self {
            Self::Name(a) => {
                if let Some((label, port)) = has_colon_port(a) {
                    let resolved = resolve_a_or_aaaa(label, None, strategy)
                        .await
                        .with_context(|| format!("resolving mx_list entry {a}"))?;
                    for mut r in resolved {
                        r.addr.set_port(port);
                        addresses.push(r);
                    }

                    return Ok(());
                }

                addresses.append(
                    &mut resolve_a_or_aaaa(a, None, strategy)
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
    treat_mx_list_as_secure: bool,
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
                .resolve_addresses(None, path_config.ip_lookup_strategy)
                .await
        } else {
            let mut addresses = vec![];
            for a in proto_config.mx_list.iter() {
                a.resolve_into(&mut addresses, path_config.ip_lookup_strategy)
                    .await?;
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
                        code: 451,
                        enhanced_code: Some(EnhancedStatusCode {
                            class: 4,
                            subject: 4,
                            detail: 4,
                        }),
                        content:
                            "KumoMTA internal: MX consisted solely of hosts on the skip_hosts list"
                                .to_string(),
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
            treat_mx_list_as_secure: proto_config.treat_mx_list_as_secure,
            recips_last_txn: HashMap::new(),
        }))
    }

    async fn attempt_connection_impl(
        &mut self,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<AttemptConnectionDisposition> {
        if let Some(client) = &mut self.client {
            if client.is_connected() {
                // If we get a unilateral response here now it can either be:
                // 1. a 421 advising us that the connection is being closed.
                // 2. a protocol sync/deviation/conformance issue, and we must
                //    therefore treat this as a closed connection.
                // Errors that present here also indicate that the connection
                // has closed.
                //
                // We want to surface these right now so that we can proceed with
                // making a new connection, rather than returning this current
                // connection that will immediately result in a transfail
                // when we try sending a message through it.
                match client.check_unilateral_response().await {
                    Ok(None) => {
                        // Still connected
                        return Ok(AttemptConnectionDisposition::ReusedExisting);
                    }
                    Ok(Some(response)) => {
                        // We got an explicit signal that the connection is closing
                        // out.  We don't consider this to be a transport error
                        // so we'll close the current connection.
                        // If we've managed to send messages so far in this session,
                        // then we're consider it to be a successful plan so we should
                        // not advance to the next candidate in the plan and make a
                        // new session.
                        // However, if we haven't managed to send anything so far,
                        // closing and restarting the plan now would likely lead to
                        // a persistent recurrence of this same state, so in that
                        // situation we need to ensure that the reconnect_strategy
                        // is applied to decide what to do.
                        tracing::debug!(
                            "{} sent a unilateral response: \
                            {response:?}, treating the connection as closed",
                            dispatcher.name
                        );
                        // Decide whether this is a successful plan
                        self.update_state_for_reconnect(dispatcher);

                        // if so, we can/should close the current session and start
                        // a new one with a fresh plan
                        if self.terminated_ok {
                            let _ = self.close_connection(dispatcher).await;
                            return Ok(
                                AttemptConnectionDisposition::PeerClosedConnectionNeedNewSession,
                            );
                        }

                        // otherwise, continue to next candidate host if that is what
                        // the reconnect_strategy indicates.
                        // While this is a return that leaves this function,
                        // our caller will typically continue its loop and call
                        // back in, but on the next call we won't show as connected
                        // and will instead reach the logic below to make a new
                        // connection in the current session, if that is what
                        // the reconnect_strategy indicated.
                        return Ok(
                            AttemptConnectionDisposition::PeerClosedConnectionContinueSession,
                        );
                    }
                    Err(err) => {
                        // We got a transport error of some kind.
                        // update_state_for_reconnect applies any reconnect_strategy
                        // which will decide whether we continue with the connection
                        // plan or whether we need to go to a new session.
                        tracing::debug!(
                            "{} had error: {err:#} \
                            while checking for liveness, treating it as closed",
                            dispatcher.name
                        );
                        self.update_state_for_reconnect(dispatcher);
                        return Ok(
                            AttemptConnectionDisposition::PeerClosedConnectionContinueSession,
                        );
                    }
                };
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
                        .enter_phase(crate::ready_queue::DispatcherPhase::ConnectionRateThrottled);
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
                            return Err(ShuttingDownError::new("waiting for max_connection_rate").into());
                        }
                    };
                } else {
                    break;
                }
            }
            dispatcher.states.lock().connection_rate_throttled.take();
            dispatcher.enter_phase(crate::ready_queue::DispatcherPhase::AttemptingConnection);
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
        dispatcher.set_detail("connect+banner");
        let (mut client, source_address) = tokio::select! {
            _ = shutdown.shutting_down() => {
                return Err(ShuttingDownError::new("waiting for new connection").into());
            }
            result = make_connection => { result? },
        }
        .with_context(|| connect_context.clone())?;
        self.source_address.replace(source_address);

        // Say EHLO/LHLO
        let helo_verb = if path_config.use_lmtp { "LHLO" } else { "EHLO" };
        dispatcher.set_detail(helo_verb);
        let pretls_caps = client
            .ehlo_lhlo(&ehlo_name, path_config.use_lmtp)
            .await
            .with_context(|| format!("{target_address}: {helo_verb} after banner"))?;

        // Use STARTTLS if available.
        let has_tls = pretls_caps.contains_key("STARTTLS");
        let broken_tls = self.has_broken_tls(&dispatcher.name);

        let mut dane_tlsa = vec![];
        let mut mta_sts_eligible = true;
        // Set when DANE published TLSA records that turned out to be unusable:
        // STARTTLS is then mandatory (the host committed to TLS) even though we
        // cannot authenticate it, so MTA-STS may add authentication but must not
        // relax the requirement back to opportunistic.
        let mut dane_requires_starttls = false;

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
            // RFC 7672 sections 2.1/2.2: DANE only applies when the chain to
            // the MX host was securely resolved. The host selection is trusted
            // when it came from a DNSSEC-validated MX RRset, or when it is a
            // locally-configured mx_list that the operator marked trusted via
            // treat_mx_list_as_secure. In either case DANE additionally requires
            // the host's address records to have been securely (DNSSEC)
            // resolved; the TLSA records are queried against the MX host
            // (RFC 7672 section 3.2.2), not the envelope/routing domain.
            let mx_selection_secure = match &dispatcher.mx {
                Some(mx) => mx.is_secure,
                // No DNS MX RRset: a locally-configured mx_list, trusted only
                // when the operator opted in via treat_mx_list_as_secure.
                None => self.treat_mx_list_as_secure,
            };
            let dane_eligible = if mx_selection_secure && address.is_secure {
                true
            } else if mx_selection_secure {
                // The MX selection was secure but the MX host's address chain
                // was not. Per RFC 7672 section 2.2.2 the host is still
                // DANE-eligible if it is a securely published CNAME alias whose
                // target merely lands in an unsigned zone; the securely
                // published TLSA RRset, not the address records, authenticates
                // the peer. An explicit CNAME lookup tells us whether that is
                // the case.
                match dns_resolver::resolve_secure_cname(&address.name).await? {
                    SecureCnameStatus::SecureAlias => {
                        self.tracer.diagnostic(Level::INFO, || {
                            format!(
                                "{} resolves via a secure CNAME into an insecure \
                                 zone; DANE remains eligible at the original name \
                                 (RFC 7672 section 2.2.2)",
                                address.name
                            )
                        });
                        true
                    }
                    SecureCnameStatus::NotSecureAlias => false,
                    SecureCnameStatus::TempFail(reason) => {
                        record_dane_result("tempfail");
                        // Downgrade resistance: when the CNAME status cannot be
                        // securely determined we must not continue without
                        // authentication. Defer instead.
                        let message = format!(
                            "DANE CNAME lookup for {} could not be securely \
                                 resolved: {reason}",
                            address.name
                        );
                        self.tracer.diagnostic(Level::INFO, || message.clone());
                        anyhow::bail!("{message}");
                    }
                }
            } else {
                false
            };

            if dane_eligible {
                match dns_resolver::resolve_dane(&address.name, port).await? {
                    DaneStatus::Records(tlsa) => {
                        record_dane_result("ok");
                        dane_tlsa = tlsa;
                        self.tracer.diagnostic(Level::INFO, || {
                            format!("DANE records for {} are: {dane_tlsa:?}", address.name)
                        });
                        enable_tls = Tls::Required;
                        // Do not use MTA-STS when we have usable DANE records
                        mta_sts_eligible = false;
                    }
                    DaneStatus::Unusable => {
                        record_dane_result("unusable");
                        // RFC 7672 section 4.1: TLSA records are published
                        // but none are usable; STARTTLS is required but we
                        // cannot authenticate the peer. The domain has no
                        // usable DANE policy, so MTA-STS may still apply as
                        // an authentication fallback (but must not relax the
                        // mandatory STARTTLS).
                        self.tracer.diagnostic(Level::INFO, || {
                            format!(
                                "DANE TLSA records for {} exist but none are usable; \
                                     requiring unauthenticated STARTTLS",
                                address.name
                            )
                        });
                        enable_tls = Tls::RequiredInsecure;
                        dane_requires_starttls = true;
                    }
                    DaneStatus::TempFail(reason) => {
                        record_dane_result("tempfail");
                        // Downgrade resistance: when the TLSA status cannot
                        // be securely determined we must not continue
                        // without authentication. Defer instead.
                        let message = format!(
                            "DANE TLSA lookup for {} could not be securely \
                                 resolved: {reason}",
                            address.name
                        );
                        self.tracer.diagnostic(Level::INFO, || message.clone());
                        anyhow::bail!("{message}");
                    }
                    DaneStatus::NotApplicable => {
                        record_dane_result("not_applicable");
                        self.tracer.diagnostic(Level::INFO, || {
                            format!("{} is not DANE-eligible", address.name)
                        });
                    }
                }
            } else {
                record_dane_result("insecure_chain");
                self.tracer.diagnostic(Level::INFO, || {
                    format!(
                        "DANE is enabled but the chain to {} is not fully \
                             DNSSEC-secure (mx_selection_secure={mx_selection_secure}, \
                             address_secure={}); not using DANE",
                        address.name, address.is_secure
                    )
                });
            }
        } else {
            self.tracer
                .diagnostic(Level::INFO, || format!("DANE is not enabled for this path"));
        }

        // Apply the TLS posture from any MTA-STS policy that was resolved
        // during MX resolution. Host gating is already structural: resolution
        // pruned disallowed hosts and isolated impossible domains, so the
        // pinned mx's policy is always satisfiable by the candidate hosts.
        // We consult it here only for TLS posture, and only when this egress
        // path opts in via enable_mta_sts. The `mta_sts_eligible` guard
        // preserves DANE precedence (DANE clears it when TLSA is present).
        if mta_sts_eligible && path_config.enable_mta_sts {
            match dispatcher.mx.as_ref().map(|mx| mx.mta_sts) {
                Some(PolicyMode::Enforce) => {
                    enable_tls = Tls::Required;
                    self.tracer.diagnostic(Level::INFO, || {
                        "MTA-STS enforce policy in effect; requiring TLS".to_string()
                    });
                }
                Some(PolicyMode::Testing) => {
                    // Don't relax a mandatory STARTTLS established by
                    // unusable DANE TLSA records.
                    if !dane_requires_starttls {
                        enable_tls = Tls::OpportunisticInsecure;
                    }
                    self.tracer.diagnostic(Level::INFO, || {
                        "MTA-STS testing policy in effect".to_string()
                    });
                }
                None | Some(PolicyMode::None) => {}
            }
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
                dispatcher.set_detail("STARTTLS");
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
                dispatcher.set_detail("STARTTLS");
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
                            client.send_command(&rfc5321::parser::Command::Quit),
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
            if !tls_enabled {
                if !path_config.allow_smtp_auth_plain_without_tls {
                    anyhow::bail!(
                        "TLS is not enabled and AUTH PLAIN is required. Skipping ({address:?}:{port})"
                    );
                }
            } else {
                // The session is encrypted; refuse to send credentials unless
                // the peer certificate was validated, otherwise an active
                // attacker could capture them.
                let validated = self
                    .tls_info
                    .as_ref()
                    .map(|info| info.authenticated)
                    .unwrap_or(false);
                if !validated && !path_config.allow_smtp_auth_plain_without_valid_certificate {
                    anyhow::bail!(
                        "TLS peer certificate was not validated and AUTH PLAIN \
                         requires a valid certificate. Skipping ({address:?}:{port})"
                    );
                }
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

            dispatcher.set_detail("AUTH PLAIN");
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
        Ok(AttemptConnectionDisposition::ConnectedNew)
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
            client
                .send_command(&rfc5321::parser::Command::Quit)
                .await
                .ok();
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

    async fn attempt_connection(
        &mut self,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<AttemptConnectionDisposition> {
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
            .sender()
            .await?
            .try_into()
            .map_err(|err: &str| anyhow::anyhow!("{err}"))?;
        let mut recipients: Vec<ForwardPath> = vec![];
        for recip in msg.recipient_list().await? {
            let recip: ForwardPath = recip
                .try_into()
                .map_err(|err: &str| anyhow::anyhow!("{err:#}"))?;
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
            .set_meta("recipient", msg.recipient_list_string().await?);
        if let Ok(name) = msg.get_queue_name().await {
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
                revised_recipient_list.push(EnvelopeAddress::from(recip));
                // The excess is ready to go immediately
                retry_immediately = true;
            }
        }

        // Race protection: the load-shedding gate may have latched
        // between obtain_message and here.  If so, do not start a
        // new SMTP transaction -- log the hold against this message
        // and let the dispatcher's normal "no more messages" path
        // (obtain_message at the top of the next loop iteration)
        // close the SMTP session.
        if let Some(reason) = crate::spool::delivery_suspension_reason() {
            let id = *msg.id();
            if let Err(err) = crate::spool::log_and_requeue_for_unhealthy_spool(
                msg,
                &dispatcher.name,
                Some(dispatcher.session_id),
                reason,
            )
            .await
            {
                tracing::error!("failed to requeue {id} while spool is unhealthy: {err:#}");
            }
            return Ok(());
        }

        dispatcher.set_detail(format!(
            "send msg with {} recip(s)",
            recipients_this_batch.len()
        ));

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
                        command: command.as_ref().map(|c| c.encode().to_string()),
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
            let queue_name = msg.get_queue_name().await?;
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
                revised_recipient_list.push(EnvelopeAddress::from(recipient.clone()));
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
            msg.set_recipient_list(revised_recipient_list).await?;
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
