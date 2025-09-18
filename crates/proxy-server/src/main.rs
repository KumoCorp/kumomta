use nix::sys::resource::{getrlimit, setrlimit, Resource};
use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;

mod proxy_handler;

/// KumoProxy SOCKS5 Proxy Server
#[derive(Debug, Parser)]
#[command(about)]
pub struct Opt {
    #[arg(long)]
    listen: Vec<String>,

    #[arg(long)]
    no_splice: bool,

    #[arg(long, default_value = "60")]
    timeout_seconds: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let opts = Opt::parse();

    if opts.listen.is_empty() {
        anyhow::bail!("No listeners defined! use the --listen option to specify at least one!");
    }

    let (_no_file_soft, no_file_hard) = getrlimit(Resource::RLIMIT_NOFILE)?;
    setrlimit(Resource::RLIMIT_NOFILE, no_file_hard, no_file_hard).context("setrlimit NOFILE")?;

    for endpoint in &opts.listen {
        start_listener(
            endpoint,
            std::time::Duration::from_secs(opts.timeout_seconds),
            opts.no_splice,
        )
        .await?;
    }

    tokio::signal::ctrl_c().await?;

    Ok(())
}

async fn start_listener(
    endpoint: &str,
    timeout: std::time::Duration,
    no_splice: bool,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(endpoint)
        .await
        .with_context(|| format!("failed to bind to {endpoint}"))?;

    let addr = listener.local_addr()?;
    log::info!("proxy listener on {addr:?}");

    tokio::spawn(async move {
        loop {
            let (socket, peer_address) = match listener.accept().await {
                Ok(tuple) => tuple,
                Err(err) => {
                    log::error!("accept failed: {err:#}");
                    return;
                }
            };

            tokio::spawn(async move {
                if let Err(err) =
                    proxy_handler::handle_proxy_client(socket, peer_address, timeout, no_splice)
                        .await
                {
                    log::error!("proxy session error: {err:#}");
                }
            });
        }
    });
    Ok(())
}
