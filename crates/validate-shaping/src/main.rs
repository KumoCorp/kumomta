use clap::Parser;
use human_bytes::human_bytes;
use kumo_api_types::shaping::{CheckLevel, Shaping, ShapingMergeOptions};
use kumo_server_memory::tracking::counted_usage;

/// KumoMTA shaping configuration validator
///
/// This utility will load the provided list of shaping files and/or URLs,
/// parse them and perform semantic validation on their contents.
///
/// This utility only considers the shaping files you provide.
/// If you want to cross check the shaping configuration with
/// the rest of your overall configuration, for example, to validate
/// that the sources defined in your shaping rules are also defined
/// by the sources helper, then you should run `kumod --validate`
/// instead of using this utility.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about)]
struct Opt {
    #[arg(required = true)]
    files: Vec<String>,

    /// Check for overlap between domain blocks and provider blocks.
    /// These are likely undesirable as they can lead to logical
    /// conflicts in the resulting configuration.
    ///
    /// Valid values are "Warn", "Error" or "Ignore".
    #[arg(long, default_value = "Warn")]
    provider_overlap: CheckLevel,

    /// Severity of DNS resolution fails for a domain.
    ///
    /// Valid values are "Warn", "Error" or "Ignore".
    #[arg(long, default_value = "Warn")]
    dns_fail: CheckLevel,

    /// How to treat a domain block when the DNS indicates that
    /// it is a NULL MX and doesn't receive mail.
    ///
    /// Valid values are "Warn", "Error" or "Ignore".
    #[arg(long, default_value = "Warn")]
    null_mx: CheckLevel,

    /// Check for aliases between domain blocks. Domains that
    /// resolve to the same site-name are likely undesirable as
    /// they can lead to logical conflicts in the resulting
    /// configuration.
    ///
    /// Valid values are "Warn", "Error" or "Ignore".
    #[arg(long, default_value = "Warn")]
    aliased_site: CheckLevel,

    /// How to treat a failure to load a remote shaping URL.
    ///
    /// Valid values are "Warn", "Error" or "Ignore".
    #[arg(long, default_value = "Warn")]
    remote_load: CheckLevel,

    /// How to treat a failure to load a local shaping file.
    ///
    /// Valid values are "Warn", "Error" or "Ignore".
    #[arg(long, default_value = "Error")]
    local_load: CheckLevel,

    /// Skip loading remote shaping URLs
    #[arg(long)]
    skip_remote: bool,
}

#[tokio::main]
async fn main() {
    let opts = Opt::parse();
    let mut failed = false;

    let shaping_opts = ShapingMergeOptions {
        provider_overlap: opts.provider_overlap,
        dns_fail: opts.dns_fail,
        null_mx: opts.null_mx,
        aliased_site: opts.aliased_site,
        skip_remote: opts.skip_remote,
        remote_load: opts.remote_load,
        local_load: opts.local_load,
    };

    let memory_start = counted_usage();
    match Shaping::merge_files(&opts.files, &shaping_opts).await {
        Ok(merged) => {
            let memory_end = counted_usage();

            for err in merged.get_errors() {
                eprintln!("ERROR: {err}");
                failed = true;
            }
            for warn in merged.get_warnings() {
                eprintln!("WARNING: {warn}");
            }
            let counted = memory_end.saturating_sub(memory_start);
            println!("INFO: approx memory used = {}", human_bytes(counted as f64));
            if !failed {
                eprintln!("OK");
            }
        }
        Err(err) => {
            eprintln!("{err:#}");
            failed = true;
        }
    }

    if failed {
        std::process::exit(1);
    }
}
