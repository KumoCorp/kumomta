use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

mod dest_site;
mod lua_config;
mod mod_kumo;
mod queue;
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
        .init();

    if let Some(policy) = opts.policy.clone() {
        lua_config::set_policy_path(policy).await?;
    }

    let mut config = lua_config::load_config().await?;
    config.async_call_callback("init", ()).await?;

    crate::spool::SpoolManager::get().await.start_spool().await;

    // FIXME: defer starting the listeners until after start_spool

    tokio::signal::ctrl_c().await?;

    Ok(())
}
