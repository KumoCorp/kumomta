use clap::Parser;
use metrics_prometheus::recorder::Layer;
use std::path::PathBuf;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

mod dest_site;
mod http_server;
mod metrics_helper;
mod mod_kumo;
mod queue;
mod runtime;
mod smtp_server;
mod spool;

#[derive(Debug, Parser)]
#[command(about = "kumo mta daemon")]
struct Opt {
    /// What to listen on
    #[arg(long, default_value = "127.0.0.1:2025")]
    listen: String,

    /// Policy file to load
    #[arg(long)]
    policy: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_env("KUMOD_LOG"))
        .with(metrics_tracing_context::MetricsLayer::new())
        .init();

    metrics::set_boxed_recorder(Box::new(
        metrics_tracing_context::TracingContextLayer::all()
            .layer(metrics_prometheus::Recorder::builder().build()),
    ))?;

    for func in [crate::mod_kumo::register, message::dkim::register] {
        config::register(func);
    }

    if let Some(policy) = opts.policy.clone() {
        config::set_policy_path(policy).await?;
    }

    let mut config = config::load_config().await?;
    config.async_call_callback("init", ()).await?;

    crate::spool::SpoolManager::get()
        .await
        .start_spool()
        .await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}
