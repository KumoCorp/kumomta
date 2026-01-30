use anyhow::Context;
use clap::Parser;
use config::CallbackSignature;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use std::io::Write;
use std::path::PathBuf;

mod metrics;
mod mod_proxy;
mod proxy_handler;

/// KumoProxy SOCKS5 Proxy Server.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// Lua policy file to load.
    /// Mutually exclusive with --listen and other legacy options.
    #[arg(long, conflicts_with_all = ["listen", "no_splice", "timeout_seconds"])]
    proxy_config: Option<PathBuf>,

    /// Directory where diagnostic log files will be placed.
    ///
    /// If omitted, diagnostics will be printed to stderr.
    #[arg(long)]
    diag_log_dir: Option<PathBuf>,

    /// How diagnostic logs render. full, compact and pretty are intended
    /// for human consumption.
    ///
    /// json outputs machine readable records.
    #[arg(long, default_value = "full")]
    diag_format: DiagnosticFormat,

    /// Instead of running the daemon, output the openapi spec json
    /// to stdout
    #[arg(long)]
    dump_openapi_spec: bool,

    // Legacy CLI options for backwards compatibility
    // These are mutually exclusive with --proxy-config
    /// [Legacy] Address(es) to listen on (e.g., "0.0.0.0:5000").
    /// Mutually exclusive with --proxy-config.
    #[arg(long, conflicts_with = "proxy_config")]
    listen: Vec<String>,

    /// [Legacy] Disable zero-copy splice optimization on Linux.
    #[arg(long, conflicts_with = "proxy_config")]
    no_splice: bool,

    /// [Legacy] Timeout in seconds for all I/O operations.
    #[arg(long, default_value = "60", conflicts_with = "proxy_config")]
    timeout_seconds: u64,
}

impl Opt {
    /// Generate a temporary Lua config file from legacy CLI options
    fn generate_legacy_config(&self) -> anyhow::Result<tempfile::NamedTempFile> {
        let mut file = tempfile::NamedTempFile::with_prefix("kumo-proxy-config-")
            .context("failed to create temporary config file")?;

        writeln!(file, "local kumo = require 'kumo'")?;
        writeln!(file)?;
        writeln!(file, "kumo.on('proxy_init', function()")?;

        for listen_addr in &self.listen {
            writeln!(file, "  kumo.start_proxy_listener {{")?;
            writeln!(file, "    listen = '{}',", listen_addr)?;
            writeln!(file, "    timeout = '{} seconds',", self.timeout_seconds)?;
            if self.no_splice {
                writeln!(file, "    use_splice = false,")?;
            }
            writeln!(file, "  }}")?;
        }

        writeln!(file, "end)")?;

        file.flush()?;
        Ok(file)
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    if opts.dump_openapi_spec {
        let router_and_docs = crate::mod_proxy::make_router();
        println!("{}", router_and_docs.docs.to_pretty_json()?);
        return Ok(());
    }

    // Validate that we have either a config file or legacy listen options
    if opts.proxy_config.is_none() && opts.listen.is_empty() {
        anyhow::bail!(
            "No configuration specified! Use --proxy-config to specify a Lua policy file, \
             or use --listen for legacy CLI mode."
        );
    }

    let (_no_file_soft, no_file_hard) = getrlimit(Resource::RLIMIT_NOFILE)?;
    setrlimit(Resource::RLIMIT_NOFILE, no_file_hard, no_file_hard).context("setrlimit NOFILE")?;

    kumo_server_common::panic::register_panic_hook();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_park(kumo_server_memory::purge_thread_cache)
        .build()
        .unwrap()
        .block_on(async move { run(opts).await })
}

async fn perform_init() -> anyhow::Result<()> {
    tracing::info!("Version is {}", version_info::kumo_version());
    let mut config = config::load_config().await?;

    let proxy_init_sig = CallbackSignature::<(), ()>::new("proxy_init");

    config
        .async_call_callback(&proxy_init_sig, ())
        .await
        .context("in proxy_init event")?;
    config.put();

    Ok(())
}

async fn signal_shutdown() {
    tracing::info!("shutting down");
}

async fn run(opts: Opt) -> anyhow::Result<()> {
    kumo_server_runtime::assign_main_runtime(tokio::runtime::Handle::current());

    // Determine the policy file to use.
    // The `legacy_file` binding holds ownership of the temp file (if any) to keep it alive.
    let legacy_file;

    let policy_path = match opts.proxy_config {
        Some(path) => path,
        None => {
            // Legacy mode: generate a temporary Lua config from CLI options
            legacy_file = opts.generate_legacy_config()?;
            legacy_file.path().to_path_buf()
        }
    };

    StartConfig {
        logging: LoggingConfig {
            log_dir: opts.diag_log_dir.clone(),
            diag_format: opts.diag_format,
            filter_env_var: "KUMO_PROXY_LOG",
            default_filter: "proxy_server=info,kumo_server_common=info,kumo_server_runtime=info",
        },
        lua_funcs: &[kumo_server_common::register, mod_proxy::register],
        policy: &policy_path,
    }
    .run(perform_init(), signal_shutdown())
    .await
}
