use crate::config::{DnsResolverConfig, NameServer, UseHostsFile};
use anyhow::Context as _;
use dns_resolver::UnboundResolver;
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The fields from kumomta's `ResolverOptions` that map to an unbound
/// configuration call. Any other field set on the public options struct
/// is rejected at configure time via serde's `deny_unknown_fields` when
/// re-deserializing the JSON form of the public options into this one.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UnboundResolverOptions {
    #[serde(default)]
    validate: Option<bool>,
    #[serde(default)]
    trust_anchor_file: Option<PathBuf>,
    #[serde(default)]
    use_hosts_file: Option<UseHostsFile>,
}

impl UnboundResolverOptions {
    fn from_public(opts: &crate::config::ResolverOptions) -> anyhow::Result<Self> {
        let value = serde_json::to_value(opts)
            .expect("ResolverOptions Serialize impl should be infallible");
        serde_json::from_value(value)
            .map_err(|e| anyhow::anyhow!("option not supported by the unbound backend: {e}"))
    }
}

pub fn build_unbound_resolver(config: &DnsResolverConfig) -> anyhow::Result<UnboundResolver> {
    let opts = UnboundResolverOptions::from_public(&config.options)?;

    let context = libunbound::Context::new()?;

    // libunbound chooses UDP/TCP internally per query and has a single
    // forwarder list, so multiple entries for the same upstream IP would
    // register that upstream more than once.
    let mut seen = BTreeSet::new();
    for ns in &config.name_servers {
        let addr = ns_socket_addr(ns)?;
        if seen.insert(addr) {
            context.set_forward(Some(addr)).context("set_forward")?;
        }
    }

    if opts.validate.unwrap_or(false) {
        context
            .add_builtin_trust_anchors()
            .context("add_builtin_trust_anchors")?;
    }

    if let Some(path) = &opts.trust_anchor_file {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("trust_anchor_file path is not valid UTF-8"))?;
        context
            .load_trust_anchor_file(path_str)
            .context("load_trust_anchor_file")?;
    }

    if !matches!(opts.use_hosts_file, Some(UseHostsFile::Never)) {
        context.load_hosts(None).context("load_hosts")?;
    }

    let context = context
        .into_async()
        .context("make async resolver context")?;
    Ok(UnboundResolver::from(context))
}

fn ns_socket_addr(ns: &NameServer) -> anyhow::Result<SocketAddr> {
    let s = match ns {
        NameServer::Ip(s) => s.as_str(),
        NameServer::Detailed { socket_addr, .. } => socket_addr.as_str(),
    };
    s.parse::<SocketAddr>()
        .with_context(|| format!("name server: '{s}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolverOptions;

    #[test]
    fn supported_options_round_trip() {
        let opts = ResolverOptions {
            validate: Some(true),
            use_hosts_file: Some(UseHostsFile::Auto),
            ..ResolverOptions::default()
        };
        let unbound_opts = UnboundResolverOptions::from_public(&opts).unwrap();
        assert_eq!(unbound_opts.validate, Some(true));
        assert!(matches!(
            unbound_opts.use_hosts_file,
            Some(UseHostsFile::Auto)
        ));
    }

    #[test]
    fn unsupported_option_is_rejected() {
        let opts = ResolverOptions {
            ndots: Some(2),
            ..ResolverOptions::default()
        };
        let err = UnboundResolverOptions::from_public(&opts)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "option not supported by the unbound backend: unknown field `ndots`, \
             expected one of `validate`, `trust_anchor_file`, `use_hosts_file`",
        );
    }
}
