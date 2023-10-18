use cidr_map::{AnyIpCidr, CidrSet};
use data_loader::KeySource;
use mlua::prelude::*;
use rfc5321::SmtpClientTimeouts;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use throttle::ThrottleSpec;

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

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct EgressPathConfig {
    #[serde(default = "EgressPathConfig::default_connection_limit")]
    pub connection_limit: usize,

    #[serde(default)]
    pub enable_tls: Tls,

    #[serde(default = "EgressPathConfig::default_enable_mta_sts")]
    pub enable_mta_sts: bool,

    #[serde(default = "EgressPathConfig::default_enable_dane")]
    pub enable_dane: bool,

    #[serde(flatten)]
    pub client_timeouts: SmtpClientTimeouts,

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
    pub max_connection_rate: Option<ThrottleSpec>,

    #[serde(default = "EgressPathConfig::default_max_deliveries_per_connection")]
    pub max_deliveries_per_connection: usize,

    #[serde(default = "EgressPathConfig::default_prohibited_hosts")]
    pub prohibited_hosts: CidrSet,

    #[serde(default)]
    pub skip_hosts: CidrSet,

    #[serde(default)]
    pub ehlo_domain: Option<String>,

    #[serde(default)]
    pub suspended: bool,
}

impl LuaUserData for EgressPathConfig {}

impl Default for EgressPathConfig {
    fn default() -> Self {
        Self {
            connection_limit: Self::default_connection_limit(),
            enable_tls: Tls::default(),
            enable_mta_sts: Self::default_enable_mta_sts(),
            enable_dane: Self::default_enable_dane(),
            max_ready: Self::default_max_ready(),
            consecutive_connection_failures_before_delay:
                Self::default_consecutive_connection_failures_before_delay(),
            smtp_port: Self::default_smtp_port(),
            max_message_rate: None,
            max_connection_rate: None,
            max_deliveries_per_connection: Self::default_max_deliveries_per_connection(),
            client_timeouts: SmtpClientTimeouts::default(),
            prohibited_hosts: Self::default_prohibited_hosts(),
            skip_hosts: CidrSet::default(),
            ehlo_domain: None,
            allow_smtp_auth_plain_without_tls: false,
            smtp_auth_plain_username: None,
            smtp_auth_plain_password: None,
            suspended: false,
        }
    }
}

impl EgressPathConfig {
    fn default_connection_limit() -> usize {
        32
    }

    fn default_enable_mta_sts() -> bool {
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

    fn default_prohibited_hosts() -> CidrSet {
        [
            AnyIpCidr::from_str("127.0.0.0/8").unwrap(),
            AnyIpCidr::from_str("::1").unwrap(),
        ]
        .into()
    }
}
