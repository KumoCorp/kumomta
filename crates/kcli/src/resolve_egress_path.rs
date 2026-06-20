use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::{ResolveEgressPathV1Request, ResolveEgressPathV1Response};
use reqwest::Url;
use std::io::Write;

/// Resolve the effective egress path configuration and throughput
/// ceilings for a destination domain and egress source.
///
/// Invokes the same `get_queue_config` and `get_egress_path_config`
/// callbacks that the live runtime would use, performs the
/// associated MX lookup, and reports the resulting configuration,
/// the derived ceilings, and the ready-queue name that would be
/// used. This is the live, server-side counterpart to the
/// `resolve-shaping-domain` script: it operates against a running
/// kumod and so reflects any policy that requires runtime state
/// (e.g. shaping helpers that read from disk at request time).
///
/// Default output is a human-readable text block. Pass `--json` for
/// the structured response, or `--config` / `--constraints` to
/// limit to one section.
#[derive(Debug, Parser)]
pub struct ResolveEgressPathCommand {
    /// The destination domain to resolve.
    pub domain: String,

    /// The egress source name. Defaults to "unspecified".
    pub source: Option<String>,

    /// Print only the TOML rendering of the egress path config.
    /// Mutually exclusive with --constraints and --json.
    #[clap(long, conflicts_with_all = ["constraints", "json"])]
    pub config: bool,

    /// Print only the human-readable ceilings block. Mutually
    /// exclusive with --config and --json.
    #[clap(long, conflicts_with_all = ["config", "json"])]
    pub constraints: bool,

    /// Print the full response as pretty JSON. Mutually exclusive
    /// with --config and --constraints.
    #[clap(long, conflicts_with_all = ["config", "constraints"])]
    pub json: bool,
}

impl ResolveEgressPathCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let response = client
            .admin_resolve_egress_path_v1(&ResolveEgressPathV1Request {
                domain: self.domain.clone(),
                source: self.source.clone(),
            })
            .await?;

        if self.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
            return Ok(());
        }

        let mut out = std::io::stdout().lock();
        if self.config {
            write!(out, "{}", mod_serde::toml_encode_pretty_compact(&response.path_config)?)?;
            return Ok(());
        }
        if self.constraints {
            write!(out, "{}", response.constraints.to_human_string())?;
            return Ok(());
        }

        render_default(&response, &mut out)?;
        Ok(())
    }
}

fn render_default(
    r: &ResolveEgressPathV1Response,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    writeln!(out, "domain: {}", r.domain)?;
    writeln!(out, "source: {}", r.source)?;
    writeln!(out, "queue:  {}", r.queue_name)?;

    writeln!(out)?;
    match &r.mx {
        Some(mx) => write!(out, "{}", mx.to_human_string())?,
        None => writeln!(out, "mx: <not resolved>")?,
    }

    writeln!(out)?;
    writeln!(out, "--- egress path config ---")?;
    writeln!(out)?;
    writeln!(out, "{}", mod_serde::toml_encode_pretty_compact(&r.path_config)?)?;

    writeln!(out, "--- effective ceilings ---")?;
    writeln!(out)?;
    write!(out, "{}", r.constraints.to_human_string())?;
    Ok(())
}
