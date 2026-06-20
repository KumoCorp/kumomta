use cidr_map::CidrSet;
use data_loader::KeySource;
use dns_resolver::IpLookupStrategy;
#[cfg(feature = "lua")]
use mlua::prelude::*;
use openssl::ssl::SslOptions;
use ordermap::OrderMap;
use rfc5321::SmtpClientTimeouts;
use rustls::crypto::aws_lc_rs::ALL_CIPHER_SUITES;
use rustls::SupportedCipherSuite;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::Write;
use std::time::Duration;
use throttle::{LimitSpec, ThrottleSpec};
use utoipa::ToSchema;

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

    pub fn is_opportunistic(&self) -> bool {
        match self {
            Self::OpportunisticInsecure | Self::Opportunistic => true,
            _ => false,
        }
    }
}

impl Default for Tls {
    fn default() -> Self {
        Self::Opportunistic
    }
}

pub fn parse_openssl_options(option_list: &str) -> anyhow::Result<SslOptions> {
    let mut result = SslOptions::empty();

    for option in option_list.split('|') {
        match SslOptions::from_name(option) {
            Some(opt) => {
                result.insert(opt);
            }
            None => {
                let mut allowed: Vec<_> = SslOptions::all()
                    .iter_names()
                    .map(|(name, _)| format!("`{name}`"))
                    .collect();
                allowed.sort();
                let allowed = allowed.join(", ");
                anyhow::bail!(
                    "`{option}` is not a valid SslOption name. \
                    Possible values are {allowed} joined together by the pipe `|` character."
                );
            }
        }
    }

    Ok(result)
}

fn deserialize_ssl_options<'de, D>(deserializer: D) -> Result<Option<SslOptions>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let maybe_options = Option::<String>::deserialize(deserializer)?;

    match maybe_options {
        None => Ok(None),
        Some(option_list) => match parse_openssl_options(&option_list) {
            Ok(options) => Ok(Some(options)),
            Err(err) => Err(D::Error::custom(format!("{err:#}"))),
        },
    }
}

fn deserialize_supported_ciphersuite<'de, D>(
    deserializer: D,
) -> Result<Vec<SupportedCipherSuite>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let suites = Vec::<String>::deserialize(deserializer)?;
    let mut result = vec![];

    for s in suites {
        match find_rustls_cipher_suite(&s) {
            Some(s) => {
                result.push(s);
            }
            None => {
                return Err(D::Error::custom(format!(
                    "`{s}` is not a valid rustls cipher suite"
                )));
            }
        }
    }

    Ok(result)
}

pub fn find_rustls_cipher_suite(name: &str) -> Option<SupportedCipherSuite> {
    for suite in ALL_CIPHER_SUITES {
        let sname = format!("{:?}", suite.suite());
        if sname.eq_ignore_ascii_case(name) {
            return Some(*suite);
        }
    }
    None
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "lua", derive(FromLua))]
pub enum WakeupStrategy {
    #[default]
    Aggressive,
    Relaxed,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "lua", derive(FromLua))]
pub enum MemoryReductionPolicy {
    #[default]
    ShrinkDataAndMeta,
    ShrinkData,
    NoShrink,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "lua", derive(FromLua))]
pub enum ConfigRefreshStrategy {
    #[default]
    Ttl,
    Epoch,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "lua", derive(FromLua))]
pub enum ReconnectStrategy {
    /// Close out the current connection session, allowing the maintainer
    /// to decide about opening a new session and starting with a fresh
    /// connection plan
    TerminateSession,
    /// Try to reconnect to the same host that we were using and where
    /// we experienced the error
    ReconnectSameHost,
    /// Advance to the next host in the connection, if any. If none remain,
    /// this is equivalent to TerminateSession
    #[default]
    ConnectNextHost,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "lua", derive(FromLua))]
#[serde(deny_unknown_fields)]
pub struct EgressPathConfig {
    #[serde(default = "EgressPathConfig::default_connection_limit")]
    pub connection_limit: LimitSpec,

    #[serde(default)]
    pub additional_connection_limits: OrderMap<String, LimitSpec>,

    #[serde(default)]
    pub enable_tls: Tls,

    #[serde(default = "EgressPathConfig::default_enable_mta_sts")]
    pub enable_mta_sts: bool,

    #[serde(default = "EgressPathConfig::default_enable_dane")]
    pub enable_dane: bool,

    #[serde(default = "EgressPathConfig::default_enable_pipelining")]
    pub enable_pipelining: bool,

    #[serde(default = "EgressPathConfig::default_enable_rset")]
    pub enable_rset: bool,

    #[serde(default)]
    pub tls_prefer_openssl: bool,

    #[serde(default)]
    pub tls_certificate: Option<KeySource>,

    #[serde(default)]
    pub tls_private_key: Option<KeySource>,

    #[serde(default)]
    pub openssl_cipher_list: Option<String>,
    #[serde(default)]
    pub openssl_cipher_suites: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_ssl_options",
        skip_serializing // FIXME
    )]
    pub openssl_options: Option<SslOptions>,

    #[serde(
        default,
        deserialize_with = "deserialize_supported_ciphersuite",
        skip_serializing // FIXME
    )]
    pub rustls_cipher_suites: Vec<SupportedCipherSuite>,

    #[serde(flatten)]
    pub client_timeouts: SmtpClientTimeouts,

    /// How long to wait for an established session to gracefully
    /// close when the system is shutting down. After this period
    /// has elapsed, sessions will be aborted.
    #[serde(default, with = "duration_serde")]
    pub system_shutdown_timeout: Option<Duration>,

    #[serde(default = "EgressPathConfig::default_max_ready")]
    pub max_ready: usize,

    #[serde(default = "EgressPathConfig::default_consecutive_connection_failures_before_delay")]
    pub consecutive_connection_failures_before_delay: usize,

    #[serde(default = "EgressPathConfig::default_smtp_port")]
    pub smtp_port: u16,

    #[serde(default)]
    pub smtp_auth_plain_username: Option<String>,

    #[serde(default)]
    pub smtp_auth_plain_password: Option<KeySource>,

    #[serde(default)]
    pub allow_smtp_auth_plain_without_tls: bool,

    #[serde(default)]
    pub max_message_rate: Option<ThrottleSpec>,

    #[serde(default)]
    pub additional_message_rate_throttles: OrderMap<String, ThrottleSpec>,

    #[serde(default)]
    pub source_selection_rate: Option<ThrottleSpec>,

    #[serde(default)]
    pub additional_source_selection_rates: OrderMap<String, ThrottleSpec>,

    #[serde(default)]
    pub max_connection_rate: Option<ThrottleSpec>,

    #[serde(default = "EgressPathConfig::default_max_deliveries_per_connection")]
    pub max_deliveries_per_connection: usize,

    #[serde(default = "EgressPathConfig::default_max_recipients_per_batch")]
    pub max_recipients_per_batch: usize,

    #[serde(default = "CidrSet::default_prohibited_hosts")]
    pub prohibited_hosts: CidrSet,

    #[serde(default)]
    pub skip_hosts: CidrSet,

    #[serde(default)]
    pub ip_lookup_strategy: IpLookupStrategy,

    #[serde(default)]
    pub ehlo_domain: Option<String>,

    // TODO: decide if we want to keep this and then document
    #[serde(default)]
    pub aggressive_connection_opening: bool,

    /// How long to wait between calls to get_egress_path_config for
    /// any given ready queue. Making this longer uses fewer
    /// resources (in aggregate) but means that it will take longer
    /// to detect and adjust to changes in the queue configuration.
    #[serde(
        default = "EgressPathConfig::default_refresh_interval",
        with = "duration_serde"
    )]
    pub refresh_interval: Duration,
    #[serde(default)]
    pub refresh_strategy: ConfigRefreshStrategy,

    #[serde(default)]
    pub dispatcher_wakeup_strategy: WakeupStrategy,
    #[serde(default)]
    pub maintainer_wakeup_strategy: WakeupStrategy,

    /// Specify an explicit provider name that should apply to this
    /// path. The provider name will be used when computing metrics
    /// rollups by provider. If omitted, then
    #[serde(default)]
    pub provider_name: Option<String>,

    /// If set, a process-local cache will be used to remember if
    /// a site has broken TLS for the duration specified.  Once
    /// encountered, we will pretend that EHLO didn't advertise STARTTLS
    /// on subsequent connection attempts.
    #[serde(default, with = "duration_serde")]
    pub remember_broken_tls: Option<Duration>,

    /// If true, when a TLS handshake fails and TLS is set to
    /// opportunistic, we will re-connect to that host with
    /// TLS disabled.
    #[serde(default)]
    pub opportunistic_tls_reconnect_on_failed_handshake: bool,

    /// If true, rather than ESMTP, use the LMTP protocol
    #[serde(default)]
    pub use_lmtp: bool,

    /// How to behave if we experience either a 421 response, an IO Error,
    /// or a timeout while talking to the peer.
    #[serde(default)]
    pub reconnect_strategy: ReconnectStrategy,

    /// Which thread pool to use for processing the ready queue
    #[serde(default)]
    pub readyq_pool_name: Option<String>,

    /// What to do to newly inserted messages when memory is low
    #[serde(default)]
    pub low_memory_reduction_policy: MemoryReductionPolicy,

    /// What to do to newly inserted messages when memory is over the soft limit
    #[serde(default)]
    pub no_memory_reduction_policy: MemoryReductionPolicy,

    /// If we experience a transport error during SMTP, should we retry the
    /// current message on the next host in the connection plan, or
    /// immediately consider it a transient failure for that message?
    #[serde(default)]
    pub try_next_host_on_transport_error: bool,

    /// If true, don't check for 8bit compatibility issues during
    /// sending, instead, leave it to the remote host to raise
    /// an error.
    #[serde(default)]
    pub ignore_8bit_checks: bool,

    /// When set, dispatcher tasks for this egress path that fail to
    /// make any forward progress for this duration are aborted by the
    /// maintainer. When omitted the effective value is derived at
    /// runtime from the protocol:
    ///   * SMTP / Xfer: max(2 * longest of mail_from, rcpt_to, data,
    ///     data_dot timeouts, 60s)
    ///   * Lua / HttpInjectionGenerator / DeferredSmtpInjection: 600s
    /// Users with a large `max_batch_latency` should set this
    /// explicitly so the watchdog does not flag batch accumulation.
    #[serde(default, with = "duration_serde")]
    pub dispatcher_progress_watchdog_timeout: Option<Duration>,
}

#[cfg(feature = "lua")]
impl LuaUserData for EgressPathConfig {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        config::impl_pairs_and_index(methods);
    }
}

impl Default for EgressPathConfig {
    fn default() -> Self {
        Self {
            connection_limit: Self::default_connection_limit(),
            tls_prefer_openssl: false,
            enable_tls: Tls::default(),
            enable_mta_sts: Self::default_enable_mta_sts(),
            enable_dane: Self::default_enable_dane(),
            enable_rset: Self::default_enable_rset(),
            enable_pipelining: Self::default_enable_pipelining(),
            max_ready: Self::default_max_ready(),
            consecutive_connection_failures_before_delay:
                Self::default_consecutive_connection_failures_before_delay(),
            smtp_port: Self::default_smtp_port(),
            max_message_rate: None,
            max_connection_rate: None,
            max_deliveries_per_connection: Self::default_max_deliveries_per_connection(),
            max_recipients_per_batch: Self::default_max_recipients_per_batch(),
            client_timeouts: SmtpClientTimeouts::default(),
            system_shutdown_timeout: None,
            prohibited_hosts: CidrSet::default_prohibited_hosts(),
            skip_hosts: CidrSet::default(),
            ehlo_domain: None,
            allow_smtp_auth_plain_without_tls: false,
            smtp_auth_plain_username: None,
            smtp_auth_plain_password: None,
            aggressive_connection_opening: false,
            rustls_cipher_suites: vec![],
            tls_certificate: None,
            tls_private_key: None,
            openssl_cipher_list: None,
            openssl_cipher_suites: None,
            openssl_options: None,
            refresh_interval: Self::default_refresh_interval(),
            refresh_strategy: ConfigRefreshStrategy::default(),
            additional_message_rate_throttles: OrderMap::default(),
            additional_connection_limits: OrderMap::default(),
            source_selection_rate: None,
            additional_source_selection_rates: OrderMap::default(),
            provider_name: None,
            remember_broken_tls: None,
            opportunistic_tls_reconnect_on_failed_handshake: false,
            use_lmtp: false,
            reconnect_strategy: ReconnectStrategy::default(),
            readyq_pool_name: None,
            low_memory_reduction_policy: MemoryReductionPolicy::default(),
            no_memory_reduction_policy: MemoryReductionPolicy::default(),
            maintainer_wakeup_strategy: WakeupStrategy::default(),
            dispatcher_wakeup_strategy: WakeupStrategy::default(),
            try_next_host_on_transport_error: false,
            ignore_8bit_checks: false,
            ip_lookup_strategy: IpLookupStrategy::default(),
            dispatcher_progress_watchdog_timeout: None,
        }
    }
}

impl EgressPathConfig {
    fn default_connection_limit() -> LimitSpec {
        LimitSpec::new(32)
    }

    fn default_enable_mta_sts() -> bool {
        true
    }

    fn default_enable_pipelining() -> bool {
        true
    }

    fn default_enable_rset() -> bool {
        true
    }

    fn default_enable_dane() -> bool {
        false
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

    fn default_max_deliveries_per_connection() -> usize {
        1024
    }

    fn default_max_recipients_per_batch() -> usize {
        100
    }

    fn default_refresh_interval() -> Duration {
        Duration::from_secs(60)
    }

    /// Compute the steady-state ceilings implied by this
    /// configuration. Per-axis, each ceiling carries a tag for which
    /// configuration term produced it, so operators can see which
    /// knob to turn.
    pub fn compute_constraints(&self) -> EgressPathConfigConstraints {
        let max_concurrent_dispatchers = {
            let mut best = EffectiveCeiling {
                value: self.connection_limit.limit as f64,
                source: CeilingSource::Primary,
                display: self.connection_limit.limit.to_string(),
            };
            for (name, spec) in &self.additional_connection_limits {
                let value = spec.limit as f64;
                if value < best.value {
                    best = EffectiveCeiling {
                        value,
                        source: CeilingSource::Additional { name: name.clone() },
                        display: spec.limit.to_string(),
                    };
                }
            }
            best
        };

        // Compute the message-rate ceiling across the primary,
        // additional throttles, and the synthetic K × C ceiling.
        let mut msg_candidates: Vec<EffectiveCeiling> = vec![];
        if let Some(spec) = &self.max_message_rate {
            msg_candidates.push(throttle_ceiling(spec, CeilingSource::Primary));
        }
        for (name, spec) in &self.additional_message_rate_throttles {
            msg_candidates.push(throttle_ceiling(
                spec,
                CeilingSource::Additional { name: name.clone() },
            ));
        }
        if let Some(spec) = &self.max_connection_rate {
            // K × C ceiling: every connection delivers at most
            // max_deliveries_per_connection messages, so total msg
            // rate cannot exceed (max_deliveries_per_connection) times
            // the connection-establishment rate. The display shows
            // both the factors and the computed total so operators
            // don't have to do the arithmetic mentally for cases
            // like 32 × 50/s.
            let k = self.max_deliveries_per_connection as u64;
            let total = ThrottleSpec {
                limit: k.saturating_mul(spec.limit),
                period: spec.period,
                max_burst: None,
                force_local: false,
            };
            msg_candidates.push(EffectiveCeiling {
                value: throttle_rate_per_sec(spec) * k as f64,
                source: CeilingSource::ReconnectCycling,
                display: format!("{k} × {spec} = {total}"),
            });
        }
        // total_cmp gives a total ordering even for NaN, so a malformed
        // candidate cannot cause a panic here. NaN sorts greater than
        // any finite value, so it naturally loses the min_by.
        let max_message_rate = msg_candidates
            .iter()
            .min_by(|a, b| a.value.total_cmp(&b.value))
            .cloned();

        // If max_message_rate is explicitly configured but a
        // different term (typically the synthetic reconnect-cycling
        // ceiling) wins the minimum, record the declared rate so
        // renderers can show an "effectively unreachable" annotation.
        // Preserve the operator's original units via ThrottleSpec's
        // Display impl.
        let max_message_rate_declared = match (&max_message_rate, &self.max_message_rate) {
            (Some(ceiling), Some(declared))
                if !matches!(ceiling.source, CeilingSource::Primary) =>
            {
                Some(declared.to_string())
            }
            _ => None,
        };

        let max_connection_rate = self
            .max_connection_rate
            .as_ref()
            .map(|spec| throttle_ceiling(spec, CeilingSource::Primary));

        let max_source_selection_rate = {
            let mut candidates: Vec<EffectiveCeiling> = vec![];
            if let Some(spec) = &self.source_selection_rate {
                candidates.push(throttle_ceiling(spec, CeilingSource::Primary));
            }
            for (name, spec) in &self.additional_source_selection_rates {
                candidates.push(throttle_ceiling(
                    spec,
                    CeilingSource::Additional { name: name.clone() },
                ));
            }
            candidates
                .into_iter()
                .min_by(|a, b| a.value.total_cmp(&b.value))
        };

        EgressPathConfigConstraints {
            max_concurrent_dispatchers,
            max_message_rate,
            max_message_rate_declared,
            max_connection_rate,
            max_source_selection_rate,
        }
    }
}

fn throttle_rate_per_sec(spec: &ThrottleSpec) -> f64 {
    spec.limit as f64 / spec.period as f64
}

fn throttle_ceiling(spec: &ThrottleSpec, source: CeilingSource) -> EffectiveCeiling {
    EffectiveCeiling {
        value: throttle_rate_per_sec(spec),
        source,
        display: spec.to_string(),
    }
}

/// Steady-state ceiling for a single throughput axis, with a tag
/// for which configuration term produced it.
///
/// {{since('dev')}}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct EffectiveCeiling {
    /// Canonical value. For rate axes: events per second; useful
    /// for numeric comparison. For concurrency: a count.
    pub value: f64,
    pub source: CeilingSource,
    /// Pre-formatted human display preserving the operator's
    /// original configuration units. A rate configured as
    /// `10000/hr` renders here as `10000/h` rather than `2.78/s`.
    /// For concurrency, the integer count. For the synthetic
    /// reconnect-cycling ceiling, the formula
    /// `max_deliveries_per_connection × <connection_rate>`.
    pub display: String,
}

/// Which configuration term produced an `EffectiveCeiling`.
///
/// {{since('dev')}}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CeilingSource {
    /// The primary configured term for this axis:
    /// `connection_limit`, `max_message_rate`,
    /// `max_connection_rate`, or `source_selection_rate`.
    Primary,
    /// A named entry from the corresponding `additional_*` map.
    Additional { name: String },
    /// Synthetic ceiling formed from
    /// `max_deliveries_per_connection × max_connection_rate`.
    /// Applies only to the message-rate axis: each connection
    /// delivers at most `max_deliveries_per_connection` messages
    /// before reconnecting, and new connections are throttled by
    /// `max_connection_rate`, so the product is a hard ceiling on
    /// system-wide message rate independent of `max_message_rate`.
    ReconnectCycling,
}

/// Steady-state ceilings implied by an `EgressPathConfig`. Each
/// ceiling carries a tag for which configuration term produced it.
///
/// {{since('dev')}}
///
/// These are per-queue ceilings; shared limits in `additional_*`
/// maps are reported at their full value and may be tighter in
/// practice when the bucket is contended.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct EgressPathConfigConstraints {
    pub max_concurrent_dispatchers: EffectiveCeiling,
    /// None when neither `max_message_rate` nor any
    /// `additional_message_rate_throttles` entry nor
    /// `max_connection_rate` is configured.
    pub max_message_rate: Option<EffectiveCeiling>,
    /// Pre-formatted display of the declared `max_message_rate` when
    /// a different term (typically `ReconnectCycling`) wins the
    /// minimum. Lets renderers show a "declared but unreachable"
    /// annotation. Uses the operator's original units.
    pub max_message_rate_declared: Option<String>,
    pub max_connection_rate: Option<EffectiveCeiling>,
    pub max_source_selection_rate: Option<EffectiveCeiling>,
}

impl EgressPathConfigConstraints {
    /// Render a human-readable multi-line block. The same formatting
    /// is used by `kcli inspect-ready-q` and by the
    /// `resolve-shaping-domain` script, so operators see the same
    /// output regardless of where they retrieved the config.
    pub fn render(&self, out: &mut dyn Write) -> std::fmt::Result {
        let label_axis = |w: &mut dyn Write, label: &str, value: &str| -> std::fmt::Result {
            writeln!(w, "  {label:<24}{value}")
        };
        let label_source =
            |w: &mut dyn Write, src: &CeilingSource, primary: &str| -> std::fmt::Result {
                let s = match src {
                    CeilingSource::Primary => primary,
                    CeilingSource::Additional { name } => name.as_str(),
                    CeilingSource::ReconnectCycling => {
                        "max_deliveries_per_connection × max_connection_rate"
                    }
                };
                writeln!(w, "    source: {s}")
            };

        writeln!(out, "ceilings:")?;

        label_axis(
            out,
            "concurrent dispatchers:",
            &self.max_concurrent_dispatchers.display,
        )?;
        label_source(
            out,
            &self.max_concurrent_dispatchers.source,
            "connection_limit",
        )?;

        if let Some(c) = &self.max_message_rate {
            label_axis(out, "message rate:", &c.display)?;
            label_source(out, &c.source, "max_message_rate")?;
            if let Some(declared) = &self.max_message_rate_declared {
                writeln!(
                    out,
                    "    declared: max_message_rate = {declared} ← effectively unreachable"
                )?;
            }
        }

        if let Some(c) = &self.max_connection_rate {
            label_axis(out, "connection rate:", &c.display)?;
            label_source(out, &c.source, "max_connection_rate")?;
        }

        if let Some(c) = &self.max_source_selection_rate {
            label_axis(out, "source selection rate:", &c.display)?;
            label_source(out, &c.source, "source_selection_rate")?;
        }

        Ok(())
    }

    pub fn to_human_string(&self) -> String {
        let mut s = String::new();
        // std::fmt::Write into a String cannot fail in practice;
        // swallow the error to avoid a panic site here.
        let _ = self.render(&mut s);
        s
    }
}

#[cfg(test)]
mod constraints_tests {
    use super::*;

    fn cfg() -> EgressPathConfig {
        EgressPathConfig::default()
    }

    /// Derive the canonical numeric value from the same display
    /// string the production code emits. A bare integer is a
    /// concurrency count; a `<limit>/<period>` form is a rate (in
    /// events per second after normalization); a `K × <rate> = <rate>`
    /// form is the reconnect-cycling formula and uses the total on
    /// the right side. Keeps tests as declarative `display`
    /// strings.
    fn value_from_display(display: &str) -> f64 {
        if let Some((_, total)) = display.split_once(" = ") {
            let spec = ThrottleSpec::try_from(total).unwrap();
            return spec.limit as f64 / spec.period as f64;
        }
        if let Ok(spec) = ThrottleSpec::try_from(display) {
            return spec.limit as f64 / spec.period as f64;
        }
        display.parse::<u64>().unwrap() as f64
    }

    fn primary(display: &str) -> EffectiveCeiling {
        EffectiveCeiling {
            value: value_from_display(display),
            source: CeilingSource::Primary,
            display: display.to_string(),
        }
    }

    fn additional(name: &str, display: &str) -> EffectiveCeiling {
        EffectiveCeiling {
            value: value_from_display(display),
            source: CeilingSource::Additional {
                name: name.to_string(),
            },
            display: display.to_string(),
        }
    }

    fn reconnect_cycling(display: &str) -> EffectiveCeiling {
        EffectiveCeiling {
            value: value_from_display(display),
            source: CeilingSource::ReconnectCycling,
            display: display.to_string(),
        }
    }

    fn throttle(s: &str) -> ThrottleSpec {
        ThrottleSpec::try_from(s).unwrap()
    }

    #[test]
    fn defaults_only_concurrency() {
        assert_eq!(
            cfg().compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: None,
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn additional_connection_limit_wins() {
        let mut p = cfg();
        p.additional_connection_limits
            .insert("provider_shared".to_string(), LimitSpec::new(10));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: additional("provider_shared", "10"),
                max_message_rate: None,
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn primary_connection_limit_wins_when_smaller() {
        let mut p = cfg();
        p.connection_limit = LimitSpec::new(4);
        p.additional_connection_limits
            .insert("large_pool".to_string(), LimitSpec::new(100));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("4"),
                max_message_rate: None,
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn primary_message_rate_only() {
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(primary("1000/s")),
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn additional_message_rate_wins() {
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        p.additional_message_rate_throttles
            .insert("provider_cap".to_string(), throttle("250/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(additional("provider_cap", "250/s")),
                max_message_rate_declared: Some("1000/s".to_string()),
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn additional_message_rate_throttles_mixed_periods() {
        // Three additional throttles with mixed periods: the smallest
        // canonical rate (10/hr ≈ 0.0028 msg/s) must win the min,
        // even though numerically 5/s has the smallest *literal*
        // limit. This exercises the period-normalized comparison.
        let mut p = cfg();
        p.additional_message_rate_throttles
            .insert("per_hour".to_string(), throttle("10/hr"));
        p.additional_message_rate_throttles
            .insert("per_minute".to_string(), throttle("8/min"));
        p.additional_message_rate_throttles
            .insert("per_second".to_string(), throttle("5/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(additional("per_hour", "10/h")),
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn hourly_rate_preserves_units() {
        // Operator-configured "10000/hr" should round-trip as
        // "10000/h" via ThrottleSpec::Display, not collapse to a
        // per-second decimal like "2.78/s". Canonical value is
        // still events per second for numeric comparison.
        let mut p = cfg();
        p.max_message_rate = Some(throttle("10000/hr"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(primary("10000/h")),
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn reconnect_cycling_wins() {
        // K = 10, C = 10/s => 100 msg/s ceiling, smaller than the
        // 1000/s max_message_rate.
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        p.max_deliveries_per_connection = 10;
        p.max_connection_rate = Some(throttle("10/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(reconnect_cycling("10 × 10/s = 100/s")),
                max_message_rate_declared: Some("1000/s".to_string()),
                max_connection_rate: Some(primary("10/s")),
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn reconnect_cycling_does_not_bind_with_large_k() {
        // K = 1024 (default), C = 10/s => 10240 msg/s synthetic,
        // not binding when max_message_rate is 1000/s. Primary wins
        // and no declared-but-unreachable annotation is needed.
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        p.max_connection_rate = Some(throttle("10/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(primary("1000/s")),
                max_message_rate_declared: None,
                max_connection_rate: Some(primary("10/s")),
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn reconnect_cycling_alone_sets_message_rate() {
        // No explicit max_message_rate, but K × C is still a
        // computable ceiling and should be reported. Nothing was
        // declared, so no annotation.
        let mut p = cfg();
        p.max_deliveries_per_connection = 5;
        p.max_connection_rate = Some(throttle("2/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: Some(reconnect_cycling("5 × 2/s = 10/s")),
                max_message_rate_declared: None,
                max_connection_rate: Some(primary("2/s")),
                max_source_selection_rate: None,
            }
        );
    }

    #[test]
    fn source_selection_rate() {
        let mut p = cfg();
        p.source_selection_rate = Some(throttle("5/s"));
        assert_eq!(
            p.compute_constraints(),
            EgressPathConfigConstraints {
                max_concurrent_dispatchers: primary("32"),
                max_message_rate: None,
                max_message_rate_declared: None,
                max_connection_rate: None,
                max_source_selection_rate: Some(primary("5/s")),
            }
        );
    }

    #[test]
    fn render_defaults() {
        let c = cfg().compute_constraints();
        k9::snapshot!(
            c.to_human_string(),
            "
ceilings:
  concurrent dispatchers: 32
    source: connection_limit

"
        );
    }

    #[test]
    fn render_primary_message_rate_no_annotation() {
        // max_message_rate is the binding term; no declared-but-
        // unreachable annotation should appear.
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        let c = p.compute_constraints();
        k9::snapshot!(
            c.to_human_string(),
            "
ceilings:
  concurrent dispatchers: 32
    source: connection_limit
  message rate:           1000/s
    source: max_message_rate

"
        );
    }

    #[test]
    fn render_reconnect_cycling_with_annotation() {
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        p.max_deliveries_per_connection = 10;
        p.max_connection_rate = Some(throttle("10/s"));
        let c = p.compute_constraints();
        k9::snapshot!(
            c.to_human_string(),
            "
ceilings:
  concurrent dispatchers: 32
    source: connection_limit
  message rate:           10 × 10/s = 100/s
    source: max_deliveries_per_connection × max_connection_rate
    declared: max_message_rate = 1000/s ← effectively unreachable
  connection rate:        10/s
    source: max_connection_rate

"
        );
    }

    #[test]
    fn render_additional_throttle_winning() {
        let mut p = cfg();
        p.max_message_rate = Some(throttle("1000/s"));
        p.additional_message_rate_throttles
            .insert("provider_cap".to_string(), throttle("250/s"));
        let c = p.compute_constraints();
        k9::snapshot!(
            c.to_human_string(),
            "
ceilings:
  concurrent dispatchers: 32
    source: connection_limit
  message rate:           250/s
    source: provider_cap
    declared: max_message_rate = 1000/s ← effectively unreachable

"
        );
    }
}
