use anyhow::Context;
use clap::Parser;
use config::CallbackSignature;
use kumo_api_types::shaping::Shaping;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

mod http_server;
mod mod_auto;

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

    /// Instead of running the daemon, output the openapi spec json
    /// to stdout
    #[arg(long)]
    dump_openapi_spec: bool,
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    if opts.dump_openapi_spec {
        let api_docs = crate::http_server::make_router().make_docs();
        println!("{}", api_docs.to_pretty_json()?);
        return Ok(());
    }

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

        // Explicitly load the shaping config now to catch silly
        // mistakes before we start up the listeners
        let sig = CallbackSignature::<(), Shaping>::new("tsa_load_shaping_data");
        let _shaping: Shaping = config
            .async_call_callback_non_default(&sig, ())
            .await
            .context("in tsa_load_shaping_data event")?;

        let tsa_init_sig = CallbackSignature::<(), ()>::new("tsa_init");

        config
            .async_call_callback(&tsa_init_sig, ())
            .await
            .context("in tsa_init event")?;

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
        lua_funcs: &[kumo_server_common::register, mod_auto::register],
        policy: &opts.policy,
    }
    .run(perform_init, signal_shutdown)
    .await
}
