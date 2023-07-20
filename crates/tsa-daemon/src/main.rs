use clap::Parser;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

/// KumoMTA Traffic Shaping Automation Daemon.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// Lua policy file to load.
    #[arg(long, default_value = "/opt/kumomta/etc/tsa/init.lua")]
    policy: PathBuf,

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

    /// Whether to enable the diagnostic tokio console
    #[arg(long)]
    tokio_console: bool,
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();
    kumo_server_common::panic::register_panic_hook();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_park(|| kumo_server_memory::purge_thread_cache())
        .build()
        .unwrap()
        .block_on(async move { run(opts).await })
}

fn perform_init() -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
    Box::pin(async move {
        let mut config = config::load_config().await?;
        config.async_call_callback("tsa_init", ()).await?;

        // TODO: start something else here?

        Ok(())
    })
}

fn signal_shutdown() -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(async move {})
}

async fn run(opts: Opt) -> anyhow::Result<()> {
    StartConfig {
        logging: LoggingConfig {
            log_dir: opts.diag_log_dir.clone(),
            diag_format: opts.diag_format,
            tokio_console: opts.tokio_console,
            filter_env_var: "KUMO_TSA_LOG",
            default_filter: "tsa_daemon=info,kumo_server_common=info",
        },
        lua_funcs: &[kumo_server_common::register],
        policy: &opts.policy,
    }
    .run(perform_init, signal_shutdown)
    .await
}
