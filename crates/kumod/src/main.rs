use crate::lifecycle::LifeCycle;
use anyhow::Context;
use caps::{CapSet, Capability, CapsHashSet};
use clap::Parser;
use metrics_prometheus::recorder::Layer;
use nix::unistd::{Uid, User};
use std::path::PathBuf;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

mod cidrset;
mod egress_path;
mod egress_source;
mod http_server;
mod lifecycle;
mod logging;
mod metrics_helper;
mod mod_kumo;
mod mx;
mod queue;
mod runtime;
mod smtp_server;
mod spool;
mod tls_helpers;

#[derive(Debug, Parser)]
#[command(about = "kumo mta daemon")]
struct Opt {
    /// Policy file to load
    #[arg(long)]
    policy: Option<PathBuf>,

    /// If started as root, which user to run as.
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

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
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
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_env("KUMOD_LOG"))
        .with(metrics_tracing_context::MetricsLayer::new())
        .init();

    metrics::set_boxed_recorder(Box::new(
        metrics_tracing_context::TracingContextLayer::all()
            .layer(metrics_prometheus::Recorder::builder().build()),
    ))?;

    for func in [
        crate::mod_kumo::register,
        message::dkim::register,
        mod_sqlite::register,
    ] {
        config::register(func);
    }

    if let Some(policy) = opts.policy.clone() {
        config::set_policy_path(policy).await?;
    }

    let mut life_cycle = LifeCycle::new();

    crate::runtime::Runtime::run(move || {
        tokio::task::spawn_local(async move {
            if let Err(err) = perform_init().await {
                tracing::error!("problem initializing: {err:#}");
                LifeCycle::request_shutdown().await;
            }
            Ok::<(), anyhow::Error>(())
        });
    })?;

    life_cycle.wait_for_shutdown().await;

    // after waiting for those to idle out, shut down logging
    crate::logging::Logger::signal_shutdown().await;

    println!("Shutdown completed OK!");
    Ok(())
}
