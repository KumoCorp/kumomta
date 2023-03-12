use crate::lifecycle::LifeCycle;
use crate::runtime::rt_spawn;
use anyhow::Context;
use caps::{CapSet, Capability, CapsHashSet};
use clap::{Parser, ValueEnum};
use metrics_prometheus::recorder::Layer;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use nix::sys::signal::{kill, SIGQUIT};
use nix::unistd::{Pid, Uid, User};
use std::path::PathBuf;
use tikv_jemallocator::Jemalloc;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod egress_path;
mod egress_source;
mod http_server;
mod lifecycle;
mod logging;
mod memory;
mod metrics_helper;
mod mod_kumo;
mod queue;
mod runtime;
mod smtp_server;
mod spool;
mod tls_helpers;

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "kebab_case")]
enum DiagnosticFormat {
    Pretty,
    Full,
    Compact,
    Json,
}

/// KumoMTA Daemon.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// Lua policy file to load.
    #[arg(long)]
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

    /// Required if started as root; specifies which user to run as once
    /// privileges have been dropped.
    ///
    /// If you truly wish to run as root,
    /// start as root and set `--user root` to make it explicit.
    #[arg(long)]
    user: Option<String>,
}

impl Opt {
    fn drop_privs(&self) -> anyhow::Result<()> {
        let uid = Uid::effective();
        if !uid.is_root() {
            if let Some(user_name) = &self.user {
                let user = User::from_name(&user_name)?
                    .ok_or_else(|| anyhow::anyhow!("Invalid user {user_name}"))?;
                if user.uid != uid {
                    anyhow::bail!(
                        "--user '{user_name}' resolves to uid {} \
                         which doesn't match your uid {uid}, and you are not root",
                        user.uid
                    );
                }
            }

            return Ok(());
        }

        let user_name = self.user.as_ref().ok_or_else(|| {
            anyhow::anyhow!("When running as root, you must set --user to the user to run as")
        })?;
        let user = User::from_name(&user_name)?
            .ok_or_else(|| anyhow::anyhow!("Invalid user {user_name}"))?;

        // We set the euid only so that we can retain CAP_NET_BIND_SERVICE
        // below. We'll still show up in the process listing as the target
        // user, but because we're dropping all the other caps, we lose all
        // other parts of our root-ness.
        nix::unistd::seteuid(user.uid).context("setuid")?;

        tracing::trace!("permitted: {:?}", caps::read(None, CapSet::Permitted)?);
        tracing::trace!("effective: {:?}", caps::read(None, CapSet::Effective)?);

        // Want to drop all capabilities except the ability to
        // bind to privileged ports, so that we can reload the
        // config and still bind to port 25
        let mut target_set = CapsHashSet::new();
        target_set.insert(Capability::CAP_NET_BIND_SERVICE);

        caps::set(None, CapSet::Effective, &target_set)
            .with_context(|| format!("setting effective caps to {target_set:?}"))?;
        caps::set(None, CapSet::Permitted, &target_set)
            .with_context(|| format!("setting permitted caps to {target_set:?}"))?;

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();
    // This MUST happen before we spawn any threads,
    // which is why we manually set up the tokio
    // runtime after we've called it.
    opts.drop_privs()?;

    let (_no_file_soft, no_file_hard) = getrlimit(Resource::RLIMIT_NOFILE)?;
    setrlimit(Resource::RLIMIT_NOFILE, no_file_hard, no_file_hard)?;

    register_panic_hook();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_park(|| crate::memory::purge_thread_cache())
        .build()
        .unwrap()
        .block_on(async move { run(opts).await })
}

async fn perform_init() -> anyhow::Result<()> {
    let mut config = config::load_config().await?;
    config.async_call_callback("init", ()).await?;

    crate::spool::SpoolManager::get().await.start_spool().await
}

async fn run(opts: Opt) -> anyhow::Result<()> {
    let (non_blocking, _non_blocking_flusher);
    let log_writer = if let Some(log_dir) = opts.diag_log_dir {
        let file_appender = tracing_appender::rolling::hourly(log_dir, "log");
        (non_blocking, _non_blocking_flusher) = tracing_appender::non_blocking(file_appender);
        BoxMakeWriter::new(non_blocking)
    } else {
        BoxMakeWriter::new(std::io::stderr)
    };

    let layer = fmt::layer().with_thread_names(true).with_writer(log_writer);
    let layer = match opts.diag_format {
        DiagnosticFormat::Pretty => layer.pretty().boxed(),
        DiagnosticFormat::Full => layer.boxed(),
        DiagnosticFormat::Compact => layer.compact().boxed(),
        DiagnosticFormat::Json => layer.json().boxed(),
    };

    tracing_subscriber::registry()
        .with(if opts.tokio_console {
            Some(console_subscriber::spawn())
        } else {
            None
        })
        .with(layer)
        .with(EnvFilter::from_env("KUMOD_LOG"))
        .with(metrics_tracing_context::MetricsLayer::new())
        .init();

    metrics::set_boxed_recorder(Box::new(
        metrics_tracing_context::TracingContextLayer::all()
            .layer(metrics_prometheus::Recorder::builder().build()),
    ))?;

    crate::memory::setup_memory_limit()?;

    for func in [
        crate::mod_kumo::register,
        message::dkim::register,
        mod_sqlite::register,
        mod_redis::register,
    ] {
        config::register(func);
    }

    config::set_policy_path(opts.policy.clone()).await?;

    let mut life_cycle = LifeCycle::new();

    let init_handle = rt_spawn("initialize".to_string(), move || {
        Ok(async move {
            let mut ok = true;
            if let Err(err) = perform_init().await {
                tracing::error!("problem initializing: {err:#}");
                LifeCycle::request_shutdown().await;
                ok = false;
            }
            // This log line is depended upon by the integration
            // test harness. Do not change or remove it without
            // making appropriate adjustments over there!
            tracing::info!("initialization complete");
            ok
        })
    })
    .await?;

    life_cycle.wait_for_shutdown().await;

    // after waiting for those to idle out, shut down logging
    crate::logging::Logger::signal_shutdown().await;

    println!("Shutdown completed OK!");

    if !init_handle.await? {
        anyhow::bail!("Initialization raised an error");
    }
    Ok(())
}

fn register_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let payload = payload.downcast_ref::<&str>().unwrap_or(&"!?");
        let bt = backtrace::Backtrace::new();
        if let Some(loc) = info.location() {
            tracing::error!(
                "panic at {}:{}:{} - {}\n{:?}",
                loc.file(),
                loc.line(),
                loc.column(),
                payload,
                bt
            );
        } else {
            tracing::error!("panic - {}\n{:?}", payload, bt);
        }

        default_hook(info);

        // Request a core dump
        kill(Pid::this(), SIGQUIT).ok();
    }));
}
