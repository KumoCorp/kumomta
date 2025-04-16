use cidr_map::CidrSet;
use data_loader::KeySource;
#[cfg(feature = "lua")]
use mlua::prelude::*;
use openssl::ssl::SslOptions;
use ordermap::OrderMap;
use rfc5321::SmtpClientTimeouts;
use rustls::crypto::aws_lc_rs::ALL_CIPHER_SUITES;
use rustls::SupportedCipherSuite;
use serde::{Deserialize, Deserializer, Serialize};
use std::time::Duration;
use throttle::{LimitSpec, ThrottleSpec};

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

    #[serde(default = "CidrSet::default_prohibited_hosts")]
    pub prohibited_hosts: CidrSet,

    #[serde(default)]
    pub skip_hosts: CidrSet,

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

    fn default_refresh_interval() -> Duration {
        Duration::from_secs(60)
    }
}
