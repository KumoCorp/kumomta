use crate::shaping_config::{assign_shaping, load_shaping, spawn_shaping_updater};
use anyhow::Context;
use clap::Parser;
use config::CallbackSignature;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use std::path::PathBuf;

mod http_server;
mod mod_auto;
mod publish;
mod shaping_config;

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

async fn perform_init() -> anyhow::Result<()> {
    let mut config = config::load_config().await?;

    // Explicitly load the shaping config now to catch silly
    // mistakes before we start up the listeners
    let shaping = load_shaping().await?;
    // and set it as the global shared copy of the shaping config
    assign_shaping(shaping);

    let tsa_init_sig = CallbackSignature::<(), ()>::new("tsa_init");

    config
        .async_call_callback(&tsa_init_sig, ())
        .await
        .context("in tsa_init event")?;

    spawn_shaping_updater()?;

    Ok(())
}

async fn signal_shutdown() {}

async fn run(opts: Opt) -> anyhow::Result<()> {
    kumo_server_runtime::assign_main_runtime(tokio::runtime::Handle::current());
    StartConfig {
        logging: LoggingConfig {
            log_dir: opts.diag_log_dir.clone(),
            diag_format: opts.diag_format,
            filter_env_var: "KUMO_TSA_LOG",
            default_filter: "tsa_daemon=info,kumo_server_common=info,kumo_server_runtime=info",
        },
        lua_funcs: &[kumo_server_common::register, mod_auto::register],
        policy: &opts.policy,
    }
    .run(perform_init(), signal_shutdown())
    .await
}
