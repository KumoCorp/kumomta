use anyhow::Context;
use caps::{CapSet, Capability, CapsHashSet};
use clap::Parser;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use kumo_server_runtime::rt_spawn;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use nix::sys::signal::{kill, SIGQUIT};
use nix::unistd::{Pid, Uid, User};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

mod delivery_metrics;
mod egress_path;
mod egress_source;
mod http_server;
mod logging;
mod lua_deliver;
mod metrics_helper;
mod mod_kumo;
mod queue;
mod ready_queue;
mod smtp_dispatcher;
mod smtp_server;
mod spool;

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

fn perform_init() -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
    Box::pin(async move {
        let mut config = config::load_config().await?;
        config.async_call_callback("init", ()).await?;

        crate::spool::SpoolManager::get()
            .await
            .start_spool()
            .await?;

        Ok(())
    })
}

async fn run(opts: Opt) -> anyhow::Result<()> {
    StartConfig {
        logging: LoggingConfig {
            log_dir: opts.diag_log_dir.clone(),
            diag_format: opts.diag_format,
            tokio_console: opts.tokio_console,
            filter_env_var: "KUMOD_LOG",
            default_filter: "kumod=info,kumo_server_common=info",
        },
        lua_funcs: &[
            kumo_server_common::register,
            crate::mod_kumo::register,
            crate::spool::register,
            crate::logging::register,
            message::dkim::register,
        ],
        policy: &opts.policy,
    }
    .run(perform_init, crate::logging::Logger::signal_shutdown)
    .await
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
