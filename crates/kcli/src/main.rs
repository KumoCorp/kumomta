use anyhow::Context;
use clap::Parser;
use reqwest::Url;
use std::time::Duration;

mod bounce;
mod bounce_cancel;
mod bounce_list;
mod inspect_message;
mod logfilter;
mod queue_summary;
mod suspend;
mod suspend_cancel;
mod suspend_list;
mod suspend_ready_q;
mod suspend_ready_q_cancel;
mod suspend_ready_q_list;
mod top;
mod trace_smtp_server;

/// KumoMTA CLI.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// URL to reach the KumoMTA HTTP API.
    /// You may set KUMO_KCLI_ENDPOINT in the environment to
    /// specify this without explicitly using --endpoint.
    /// If not specified, http://127.0.0.1:8000 will be assumed.
    #[arg(long)]
    endpoint: Option<String>,

    #[command(subcommand)]
    cmd: SubCommand,
}

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Parser)]
enum SubCommand {
    Bounce(bounce::BounceCommand),
    BounceList(bounce_list::BounceListCommand),
    BounceCancel(bounce_cancel::BounceCancelCommand),
    Suspend(suspend::SuspendCommand),
    SuspendList(suspend_list::SuspendListCommand),
    SuspendCancel(suspend_cancel::SuspendCancelCommand),
    SuspendReadyQ(suspend_ready_q::SuspendReadyQCommand),
    SuspendReadyQList(suspend_ready_q_list::SuspendReadyQListCommand),
    SuspendReadyQCancel(suspend_ready_q_cancel::SuspendReadyQCancelCommand),
    SetLogFilter(logfilter::SetLogFilterCommand),
    InspectMessage(inspect_message::InspectMessageCommand),
    QueueSummary(queue_summary::QueueSummaryCommand),
    TraceSmtpServer(trace_smtp_server::TraceSmtpServerCommand),
    Top(top::TopCommand),
}

impl SubCommand {
    async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        match self {
            Self::Bounce(cmd) => cmd.run(endpoint).await,
            Self::BounceCancel(cmd) => cmd.run(endpoint).await,
            Self::BounceList(cmd) => cmd.run(endpoint).await,
            Self::Suspend(cmd) => cmd.run(endpoint).await,
            Self::SuspendCancel(cmd) => cmd.run(endpoint).await,
            Self::SuspendList(cmd) => cmd.run(endpoint).await,
            Self::SuspendReadyQ(cmd) => cmd.run(endpoint).await,
            Self::SuspendReadyQCancel(cmd) => cmd.run(endpoint).await,
            Self::SuspendReadyQList(cmd) => cmd.run(endpoint).await,
            Self::SetLogFilter(cmd) => cmd.run(endpoint).await,
            Self::InspectMessage(cmd) => cmd.run(endpoint).await,
            Self::QueueSummary(cmd) => cmd.run(endpoint).await,
            Self::TraceSmtpServer(cmd) => cmd.run(endpoint).await,
            Self::Top(cmd) => cmd.run(endpoint).await,
        }
    }
}

pub async fn post<T: reqwest::IntoUrl, B: serde::Serialize>(
    url: T,
    body: &B,
) -> reqwest::Result<reqwest::Response> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()?
        .post(url)
        .json(body)
        .send()
        .await
}

pub async fn json_body<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let data = response.bytes().await.context("ready response body")?;
    serde_json::from_slice(&data).with_context(|| {
        format!(
            "parsing response as json: {}",
            String::from_utf8_lossy(&data)
        )
    })
}

pub async fn request_with_text_response<T: reqwest::IntoUrl, B: serde::Serialize>(
    method: reqwest::Method,
    url: T,
    body: &B,
) -> anyhow::Result<String> {
    let response = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()?
        .request(method, url)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    let body_bytes = response.bytes().await.with_context(|| {
        format!(
            "request status {}: {}, and failed to read response body",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )
    })?;
    let body_text = String::from_utf8_lossy(&body_bytes);
    if !status.is_success() {
        anyhow::bail!(
            "request status {}: {}. Response body: {body_text}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
        );
    }

    Ok(body_text.to_string())
}

pub async fn request_with_json_response<
    T: reqwest::IntoUrl,
    B: serde::Serialize,
    R: serde::de::DeserializeOwned,
>(
    method: reqwest::Method,
    url: T,
    body: &B,
) -> anyhow::Result<R> {
    let response = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()?
        .request(method, url)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body_bytes = response.bytes().await.with_context(|| {
            format!(
                "request status {}: {}, and failed to read response body",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })?;
        anyhow::bail!(
            "request status {}: {}. Response body: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            String::from_utf8_lossy(&body_bytes)
        );
    }
    json_body(response).await.with_context(|| {
        format!(
            "request status {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let endpoint = opts
        .endpoint
        .or_else(|| std::env::var("KUMO_KCLI_ENDPOINT").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8000".to_string());

    let url = Url::parse(&endpoint)?;
    opts.cmd.run(&url).await
}
