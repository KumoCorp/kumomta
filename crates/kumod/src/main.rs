use anyhow::{Context, Result};
use clap::{Parser, ValueEnum, ValueHint};
use tokio::net::{TcpListener, TcpStream};
use tracing::{event, instrument, Level};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Parser)]
#[command(about = "kumo mta daemon")]
struct Opt {
    /// What to listen on
    #[arg(long, default_value = "127.0.0.1:2025")]
    listen: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_env("KUMOD_LOG"))
        .init();

    let listener = TcpListener::bind(&opts.listen)
        .await
        .with_context(|| format!("failed to bind to {}", opts.listen))?;

    println!("Listening on {}", opts.listen);

    loop {
        // The second item contains the IP and port of the new connection.
        let (socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            process(socket).await;
        });
    }
}

#[instrument(
    skip(socket),
    fields(
        addr=%socket.local_addr().unwrap(),
        peer_addr=%socket.peer_addr().unwrap()
    )
)]
async fn process(socket: TcpStream) {
    println!("Do something with client here");
    inner().await;
}

#[instrument]
async fn inner() {
    event!(Level::TRACE, "I am inner");
}
