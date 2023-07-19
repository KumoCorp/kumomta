use crate::lifecycle::LifeCycle;
use anyhow::Context;
use caps::{CapSet, Capability, CapsHashSet};
use clap::{Parser, ValueEnum};
use kumo_server_runtime::rt_spawn;
use metrics_prometheus::recorder::Layer as _;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use nix::sys::signal::{kill, SIGQUIT};
use nix::unistd::{Pid, Uid, User};
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer};

// Why in the heck is this a function and not simply the reload handle itself?
// The reason is because the tracing_subscriber crate makes heavy use of composed
// generic types and with the configuration we have chosen, some of the layers have
// `impl Layer` types that cannot be named here.
// <https://en.wiktionary.org/wiki/Voldemort_type>
//
// Even if it could be named, writing out its type here would make your eyes bleed.
// Even the rust compiler doesn't want to print the name, instead writing it out
// to a separate debugging file in its diagnostics!
//
// The approach taken is to stash a closure into this, and the closure capture
// the reload handle and operates upon it.
//
// This way we don't need to name the type, and won't need to struggle with re-naming
// it if we change the layering of the log subscriber.
static TRACING_FILTER_RELOAD_HANDLE: OnceCell<
    Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>,
> = OnceCell::new();

mod delivery_metrics;
mod egress_path;
mod egress_source;
mod http_server;
mod lifecycle;
mod logging;
mod lua_deliver;
mod metrics_helper;
mod mod_kumo;
mod queue;
mod ready_queue;
mod smtp_dispatcher;
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
    #[arg(long, default_value = "/opt/kumomta/etc/policy/init.lua")]
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

        nix::unistd::setgid(user.gid).context("setgid")?;
        // We set the euid only so that we can retain CAP_NET_BIND_SERVICE
        // below. We'll still show up in the process listing as the target
        // user, but because we're dropping all the other caps, we lose all
        // other parts of our root-ness.
        nix::unistd::seteuid(user.uid).context("setuid")?;

        // eprintln!("permitted: {:?}", caps::read(None, CapSet::Permitted)?);
        // eprintln!("effective: {:?}", caps::read(None, CapSet::Effective)?);

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
        .on_thread_park(|| kumo_server_memory::purge_thread_cache())
        .build()
        .unwrap()
        .block_on(async move { run(opts).await })
}

async fn perform_init() -> anyhow::Result<()> {
    let mut config = config::load_config().await?;
    config.async_call_callback("init", ()).await?;

    crate::spool::SpoolManager::get().await.start_spool().await
}

pub fn set_diagnostic_log_filter(new_filter: &str) -> anyhow::Result<()> {
    let func = TRACING_FILTER_RELOAD_HANDLE
        .get()
        .ok_or_else(|| anyhow::anyhow!("unable to retrieve filter reload handle"))?;
    (func)(new_filter)
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

    let env_filter = EnvFilter::try_new(
        std::env::var("KUMOD_LOG")
            .as_deref()
            .unwrap_or("kumod=info"),
    )?;
    let (env_filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
    tracing_subscriber::registry()
        .with(if opts.tokio_console {
            Some(console_subscriber::spawn())
        } else {
            None
        })
        .with(layer.with_filter(env_filter))
        .with(metrics_tracing_context::MetricsLayer::new())
        .init();

    TRACING_FILTER_RELOAD_HANDLE
        .set(Box::new(move |new_filter: &str| {
            let f = EnvFilter::try_new(new_filter)
                .with_context(|| format!("parsing log filter '{new_filter}'"))?;
            Ok(reload_handle.reload(f).context("applying new log filter")?)
        }))
        .map_err(|_| anyhow::anyhow!("failed to assign reloadable logging filter"))?;

    metrics::set_boxed_recorder(Box::new(
        metrics_tracing_context::TracingContextLayer::all()
            .layer(metrics_prometheus::Recorder::builder().build()),
    ))?;

    kumo_server_memory::setup_memory_limit()?;

    for func in [
        crate::mod_kumo::register,
        data_loader::register,
        domain_map::register,
        message::dkim::register,
        mod_amqp::register,
        mod_http::register,
        mod_sqlite::register,
        mod_redis::register,
        mod_dns_resolver::register,
        mod_memoize::register,
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

    tracing::info!("Shutdown completed OK!");

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
