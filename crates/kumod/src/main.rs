use anyhow::Context;
use chrono::Utc;
use clap::Parser;
use config::CallbackSignature;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use kumo_server_lifecycle::LifeCycle;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use nix::unistd::{Uid, User};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::LazyLock;

pub static PRE_INIT_SIG: LazyLock<CallbackSignature<(), ()>> =
    LazyLock::new(|| CallbackSignature::new_with_multiple("pre_init"));
pub static VALIDATE_SIG: LazyLock<CallbackSignature<(), ()>> =
    LazyLock::new(|| CallbackSignature::new_with_multiple("validate_config"));

mod accounting;
mod delivery_metrics;
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
mod spf;
mod spool;

/// KumoMTA Daemon.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Clone, Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// Lua policy file to load.
    #[arg(long, default_value = "/opt/kumomta/etc/policy/init.lua")]
    policy: PathBuf,

    /// When set, run the policy init function in validation mode,
    /// then stop. In validation mode, listeners are not started
    /// and the spool is not acquired.
    #[arg(long)]
    validate: bool,

    /// Rather than spawning kumod in service mode, execute
    /// the policy script as a standalone script and then exit.
    /// Use kumo.on('main') to define the entrypoint for the script
    /// and receive the arguments from --script-args.
    #[arg(long)]
    script: bool,

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
    /// to stdout.
    #[arg(long)]
    dump_openapi_spec: bool,

    /// Required if started as root; specifies which user to run as once
    /// privileges have been dropped.
    ///
    /// If you truly wish to run as root,
    /// start as root and set `--user root` to make it explicit.
    #[arg(long)]
    user: Option<String>,

    /// Deprecated: List of arguments to pass to the `main` event when
    /// running in --script mode. Can be used multiple times.
    ///
    /// `kumod --script --script-args foo --script-args bar`
    ///
    /// is the deprecated equivalent to the preferred:
    ///
    /// `kumod --script -- foo bar`
    ///
    /// and is preserved for legacy compatibility.
    #[arg(long = "script-args", requires("script"))]
    legacy_script_args: Vec<String>,

    /// List of arguments to pass to the `main` event when
    /// running in --script mode.
    #[arg(last(true), requires("script"))]
    script_args: Vec<String>,
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

        #[cfg(target_os = "linux")]
        {
            // eprintln!("permitted: {:?}", caps::read(None, CapSet::Permitted)?);
            // eprintln!("effective: {:?}", caps::read(None, CapSet::Effective)?);

            // Want to drop all capabilities except the ability to
            // bind to privileged ports, so that we can reload the
            // config and still bind to port 25
            use caps::{CapSet, Capability, CapsHashSet};
            let mut target_set = CapsHashSet::new();
            target_set.insert(Capability::CAP_NET_BIND_SERVICE);

            caps::set(None, CapSet::Effective, &target_set)
                .with_context(|| format!("setting effective caps to {target_set:?}"))?;
            caps::set(None, CapSet::Permitted, &target_set)
                .with_context(|| format!("setting permitted caps to {target_set:?}"))?;
        }

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    if opts.dump_openapi_spec {
        let api_docs = crate::http_server::make_router().make_docs();
        println!("{}", api_docs.to_pretty_json()?);
        return Ok(());
    }

    // This MUST happen before we spawn any threads,
    // which is why we manually set up the tokio
    // runtime after we've called it.
    opts.drop_privs().context("drop_privs")?;

    let (_no_file_soft, no_file_hard) = getrlimit(Resource::RLIMIT_NOFILE)?;
    setrlimit(Resource::RLIMIT_NOFILE, no_file_hard, no_file_hard).context("setrlimit NOFILE")?;

    kumo_server_common::panic::register_panic_hook();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_park(|| kumo_server_memory::purge_thread_cache())
        .event_interval(
            std::env::var("KUMOD_EVENT_INTERVAL")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(61),
        )
        .max_io_events_per_tick(
            std::env::var("KUMOD_IO_EVENTS_PER_TICK")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(1024),
        )
        .max_blocking_threads(
            std::env::var("KUMOD_MAX_BLOCKING_THREADS")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(512),
        )
        .build()
        .unwrap()
        .block_on(async move { run(opts).await })?;
    tracing::info!("application logic complete, returning from main");
    Ok(())
}

fn perform_init(opts: Opt) -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
    Box::pin(async move {
        let nodeid = kumo_server_common::nodeid::NodeId::get();
        tracing::info!("NodeId is {nodeid}");
        let start_time = Utc::now();

        let mut config = config::load_config().await.context("load_config")?;

        if opts.script {
            // Convert the list of strings into a MultiValue so that
            // the event handler can be like:
            // `kumo.on('main', function(arg1, arg2)`
            // rather than receiving an array and manually unpacking
            // the argument list
            #[derive(Clone)]
            struct ParamList(Vec<String>);
            impl<'lua> mlua::IntoLuaMulti<'lua> for ParamList {
                fn into_lua_multi(
                    self,
                    lua: &'lua mlua::Lua,
                ) -> mlua::Result<mlua::MultiValue<'lua>> {
                    let mut args = vec![];
                    for arg in self.0 {
                        args.push(mlua::Value::String(lua.create_string(arg)?));
                    }
                    Ok(mlua::MultiValue::from_vec(args))
                }
            }

            let main_sig = CallbackSignature::<ParamList, ()>::new("main");

            let mut script_args = opts.legacy_script_args;
            script_args.append(&mut opts.script_args.clone());

            config
                .async_call_callback(&main_sig, ParamList(script_args))
                .await
                .context("call main callback")?;
            LifeCycle::request_shutdown().await;
            return Ok(());
        }

        config
            .async_call_callback(&PRE_INIT_SIG, ())
            .await
            .context("call pre_init callback")?;

        let init_sig = CallbackSignature::<(), ()>::new("init");
        config
            .async_call_callback(&init_sig, ())
            .await
            .context("call init callback")?;

        if opts.validate {
            config
                .async_call_callback(&VALIDATE_SIG, ())
                .await
                .context("call validate_config callback")?;

            if config::validation_failed() {
                anyhow::bail!("Validation failed");
            }

            LifeCycle::request_shutdown().await;
        } else {
            crate::spool::SpoolManager::get()
                .start_spool(start_time)
                .await
                .context("start_spool")?;

            lruttl::spawn_memory_monitor();
            config::epoch::start_monitor();
        }

        Ok(())
    })
}

async fn run(opts: Opt) -> anyhow::Result<()> {
    kumo_server_runtime::assign_main_runtime(tokio::runtime::Handle::current());
    config::VALIDATE_ONLY.store(opts.validate, std::sync::atomic::Ordering::Relaxed);

    let res = StartConfig {
        logging: LoggingConfig {
            log_dir: opts.diag_log_dir.clone(),
            diag_format: opts.diag_format,
            filter_env_var: "KUMOD_LOG",
            default_filter: if opts.validate || opts.script {
                ""
            } else {
                "kumod=info,config=info,kumo_server_common=info,kumo_server_runtime=info,lruttl=info"
            },
        },
        lua_funcs: &[
            kumo_server_common::register,
            crate::mod_kumo::register,
            crate::spool::register,
            crate::logging::register,
            message::dkim::register,
            crate::spf::register,
        ],
        policy: &opts.policy,
    }
    .run(
        {
            let opts = opts.clone();
            move || perform_init(opts)
        },
        crate::logging::Logger::signal_shutdown,
    )
    .await;

    if let Err(err) = crate::accounting::ACCT.flush() {
        tracing::error!("error flushing ACCT: {err:#}");
    }

    if let Err(err) = crate::spool::SpoolManager::shutdown().await {
        tracing::error!("error shutting down spool: {err:#}");
    }

    res
}
