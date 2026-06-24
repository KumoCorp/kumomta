//! Public Lua-facing DNS resolver configuration types.
//!
//! Owned by kumomta so that upgrades of the underlying resolver backends
//! do not require breaking changes to user configs.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DnsResolverConfig {
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub search: Vec<String>,
    #[serde(default)]
    pub name_servers: Vec<NameServer>,
    #[serde(default)]
    pub options: ResolverOptions,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum NameServer {
    Ip(String),
    Detailed {
        socket_addr: String,
        #[serde(default)]
        protocol: Protocol,
        #[serde(default = "default_trust_negative_responses")]
        trust_negative_responses: bool,
        #[serde(default)]
        bind_addr: Option<String>,
    },
}

fn default_trust_negative_responses() -> bool {
    true
}

/// `UdpThenTcp` is the default because it preserves same-server TCP fallback
/// for truncated UDP responses (TC bit) and large records; falling through
/// the global server-selection logic instead would add latency and could
/// route a TC-bit retry to a different upstream.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Protocol {
    Udp,
    Tcp,
    #[default]
    UdpThenTcp,
}

// `skip_serializing_if = "Option::is_none"` keeps the serialized form down to
// fields the user actually set, which the unbound backend relies on when
// re-deserializing into its own narrower options struct.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolverOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ndots: Option<usize>,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edns0: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_strategy: Option<IpStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_hosts_file: Option<UseHostsFile>,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub positive_min_ttl: Option<Duration>,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub negative_min_ttl: Option<Duration>,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub positive_max_ttl: Option<Duration>,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub negative_max_ttl: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_concurrent_reqs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_intermediates: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub try_tcp_on_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_ordering_strategy: Option<ServerOrderingStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursion_desired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_randomization: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_anchor_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub enum IpStrategy {
    Ipv4Only,
    Ipv6Only,
    Ipv4AndIpv6,
    Ipv6AndIpv4,
    Ipv6thenIpv4,
    Ipv4thenIpv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub enum UseHostsFile {
    Always,
    Auto,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub enum ServerOrderingStrategy {
    QueryStatistics,
    RoundRobin,
    UserProvidedOrder,
}
