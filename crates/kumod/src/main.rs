use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

mod lua_config;
mod smtp_server;

use crate::smtp_server::SmtpServer;

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

    let listener = TcpListener::bind(&opts.listen)
        .await
        .with_context(|| format!("failed to bind to {}", opts.listen))?;

    println!("Listening on {}", opts.listen);

    loop {
        // The second item contains the IP and port of the new connection.
        let (socket, _) = listener.accept().await?;
        if let Err(err) = SmtpServer::run(socket).await {
            error!("Error in SmtpServer: {err:#}");
        }
    }
}
