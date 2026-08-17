use crate::config::{DnsResolverConfig, NameServer, TrustAnchorFile, UseHostsFile};
use anyhow::Context as _;
use dns_resolver::UnboundResolver;
use std::collections::BTreeSet;
use std::net::SocketAddr;

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
    trust_anchor_file: Option<TrustAnchorFile>,
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

    match &opts.trust_anchor_file {
        // No anchor file: validate against the bundled root trust anchors.
        None => {
            if opts.validate.unwrap_or(false) {
                context
                    .add_builtin_trust_anchors()
                    .context("add_builtin_trust_anchors")?;
            }
        }
        // Static file: loaded in addition to the bundled anchors (when
        // validation is enabled), and never updated.
        Some(TrustAnchorFile::Static(path)) => {
            if opts.validate.unwrap_or(false) {
                context
                    .add_builtin_trust_anchors()
                    .context("add_builtin_trust_anchors")?;
            }
            context
                .load_trust_anchor_file(path)
                .context("load_trust_anchor_file")?;
        }
        // Managed file: RFC 5011 auto-maintained. The file is the sole anchor
        // source (no bundled anchors); seed it with the bundled anchors if it
        // is missing or empty so validation works immediately and RFC 5011
        // keeps it current thereafter.
        Some(TrustAnchorFile::Managed { managed }) => {
            seed_managed_anchor_file(managed)?;
            context
                .load_trust_anchor_file_with_auto_update(managed)
                .context("load_trust_anchor_file_with_auto_update")?;
        }
    }

    if !matches!(opts.use_hosts_file, Some(UseHostsFile::Never)) {
        context.load_hosts(None).context("load_hosts")?;
    }

    let context = context
        .into_async()
        .context("make async resolver context")?;
    Ok(UnboundResolver::from(context))
}

/// Seed an RFC 5011 managed trust anchor file with the bundled root anchors if
/// it does not yet exist (or is empty). This gives unbound a valid starting
/// anchor so DNSSEC validation works on first use; RFC 5011 then maintains the
/// file across future root KSK rollovers.
fn seed_managed_anchor_file(path: &str) -> anyhow::Result<()> {
    let needs_seed = match std::fs::metadata(path) {
        Ok(meta) => meta.len() == 0,
        Err(_) => true,
    };
    if needs_seed {
        std::fs::write(path, libunbound::ROOT_TRUST_ANCHORS.join("\n"))
            .with_context(|| format!("seeding managed trust anchor file {path:?}"))?;
    }
    Ok(())
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
