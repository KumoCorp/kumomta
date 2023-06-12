use clap::Parser;
use reqwest::Url;

mod bounce;
mod bounce_cancel;
mod bounce_list;
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
    BounceList(bounce_list::BounceListCommand),
    BounceCancel(bounce_cancel::BounceCancelCommand),
    SetLogFilter(logfilter::SetLogFilterCommand),
}

impl SubCommand {
    async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        match self {
            Self::Bounce(cmd) => cmd.run(endpoint).await,
            Self::BounceCancel(cmd) => cmd.run(endpoint).await,
            Self::BounceList(cmd) => cmd.run(endpoint).await,
            Self::SetLogFilter(cmd) => cmd.run(endpoint).await,
        }
    }
}

pub async fn post<T: reqwest::IntoUrl, B: serde::Serialize>(
    url: T,
    body: &B,
) -> reqwest::Result<reqwest::Response> {
    reqwest::Client::builder()
        .build()?
        .post(url)
        .json(body)
        .send()
        .await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let url = Url::parse(&opts.endpoint)?;
    opts.cmd.run(&url).await
}
