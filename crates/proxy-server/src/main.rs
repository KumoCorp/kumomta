use anyhow::Context;
use clap::Parser;
use tokio::net::TcpListener;

mod proxy_handler;
#[cfg(target_os = "linux")]
mod splice_copy;

/// KumoProxy SOCKS5 Proxy Server
#[derive(Debug, Parser)]
#[command(about)]
pub struct Opt {
    #[arg(long)]
    listen: Vec<String>,

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

    for endpoint in &opts.listen {
        start_listener(
            endpoint,
            std::time::Duration::from_secs(opts.timeout_seconds),
        )
        .await?;
    }

    tokio::signal::ctrl_c().await?;

    Ok(())
}

async fn start_listener(endpoint: &str, timeout: std::time::Duration) -> anyhow::Result<()> {
    let listener = TcpListener::bind(endpoint)
        .await
        .with_context(|| format!("failed to bind to {endpoint}"))?;

    let addr = listener.local_addr()?;
    log::info!("proxy listener on {addr:?}");

    tokio::spawn(async move {
        loop {
            let (socket, peer_address) = listener.accept().await.context("accepting connection")?;
            tokio::spawn(async move {
                proxy_handler::handle_proxy_client(socket, peer_address, timeout).await
            });
        }

        #[allow(unreachable_code)]
        anyhow::Result::<()>::Ok(())
    });
    Ok(())
}
