use clap::Parser;
use reqwest::{ClientBuilder, RequestBuilder, Url};

mod bounce;
mod logfilter;

/// KumoMTA CLI.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// URL to reach the KumoMTA HTTP API
    #[arg(long)]
    endpoint: String,

    #[command(subcommand)]
    cmd: SubCommand,
}

#[derive(Debug, Parser)]
enum SubCommand {
    Bounce(bounce::BounceCommand),
    SetLogFilter(logfilter::SetLogFilterCommand),
}

impl SubCommand {
    async fn run(&self, request: RequestBuilder) -> anyhow::Result<()> {
        match self {
            Self::Bounce(cmd) => cmd.run(request).await,
            Self::SetLogFilter(cmd) => cmd.run(request).await,
        }
    }

    fn url(&self, endpoint: Url) -> anyhow::Result<Url> {
        match self {
            Self::Bounce(cmd) => cmd.url(endpoint),
            Self::SetLogFilter(cmd) => cmd.url(endpoint),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let url = Url::parse(&opts.endpoint)?;
    let url = opts.cmd.url(url)?;

    let client = ClientBuilder::new().build()?.post(url);

    opts.cmd.run(client).await
}
