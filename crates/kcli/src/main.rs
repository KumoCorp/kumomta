use anyhow::Context;
use clap::{Parser, ValueEnum};
use futures::Stream;
use reqwest::Url;
use std::collections::HashMap;
use std::time::Duration;

mod bounce;
mod bounce_cancel;
mod bounce_list;
mod inspect_message;
mod inspect_sched_q;
mod logfilter;
mod provider_summary;
mod queue_summary;
mod rebind;
mod suspend;
mod suspend_cancel;
mod suspend_list;
mod suspend_ready_q;
mod suspend_ready_q_cancel;
mod suspend_ready_q_list;
mod top;
mod trace_smtp_client;
mod trace_smtp_server;
mod xfer;
mod xfer_cancel;

/// KumoMTA CLI.
///
/// Interacts with a KumoMTA instance via its HTTP API endpoint.
/// To use it, you must be running an HTTP listener.
///
/// The default is to assume that KumoMTA is running a listener
/// at http://127.0.0.1:8000 (which is in the default configuration),
/// but otherwise you can override this either via the --endpoint
/// parameter or KUMO_KCLI_ENDPOINT environment variable.
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
    #[command(hide = true)]
    MarkdownHelp,
    Bounce(bounce::BounceCommand),
    BounceList(bounce_list::BounceListCommand),
    BounceCancel(bounce_cancel::BounceCancelCommand),
    Rebind(rebind::RebindCommand),
    Suspend(suspend::SuspendCommand),
    SuspendList(suspend_list::SuspendListCommand),
    SuspendCancel(suspend_cancel::SuspendCancelCommand),
    SuspendReadyQ(suspend_ready_q::SuspendReadyQCommand),
    SuspendReadyQList(suspend_ready_q_list::SuspendReadyQListCommand),
    SuspendReadyQCancel(suspend_ready_q_cancel::SuspendReadyQCancelCommand),
    SetLogFilter(logfilter::SetLogFilterCommand),
    InspectMessage(inspect_message::InspectMessageCommand),
    InspectSchedQ(inspect_sched_q::InspectQueueCommand),
    ProviderSummary(provider_summary::ProviderSummaryCommand),
    QueueSummary(queue_summary::QueueSummaryCommand),
    TraceSmtpClient(trace_smtp_client::TraceSmtpClientCommand),
    TraceSmtpServer(trace_smtp_server::TraceSmtpServerCommand),
    Top(top::TopCommand),
    Xfer(xfer::XferCommand),
    XferCancel(xfer_cancel::XferCancelCommand),
}

impl SubCommand {
    async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        match self {
            Self::MarkdownHelp => {
                use clap::CommandFactory;
                use clap_markdown::{help_markdown_command_custom, MarkdownOptions};

                let options = MarkdownOptions::new()
                    .show_footer(false)
                    .show_table_of_contents(false);

                let cmd = Opt::command();
                let overall_help = help_markdown_command_custom(&cmd, &options);

                let doc_tags: &[(&str, &[&str])] = &[
                    ("bounce", &["bounce"]),
                    ("bounce-list", &["bounce"]),
                    ("bounce-cancel", &["bounce"]),
                    ("suspend", &["suspend"]),
                    ("suspend-list", &["suspend"]),
                    ("suspend-cancel", &["suspend"]),
                    ("suspend-ready-q", &["suspend"]),
                    ("suspend-ready-q-list", &["suspend"]),
                    ("suspend-ready-q-cancel", &["suspend"]),
                    ("set-log-filter", &["logging", "debugging"]),
                    ("inspect-message", &["message", "debugging"]),
                    ("inspect-sched-q", &["debugging"]),
                    ("provider-summary", &["ops"]),
                    ("queue-summary", &["ops"]),
                    ("trace-smtp-client", &["ops", "debugging"]),
                    ("trace-smtp-server", &["ops", "debugging"]),
                    ("top", &["ops", "debugging"]),
                    ("xfer", &["ops", "xfer"]),
                    ("xfer-cancel", &["ops", "xfer"]),
                ];
                let doc_tags: HashMap<&str, &[&str]> =
                    doc_tags.into_iter().map(|(k, v)| (*k, &v[..])).collect();

                // We want a separate markdown page per sub-command, so we're
                // doing a bit of grubbing around to split that out here

                for (idx, chunk) in overall_help.split("## `kcli ").enumerate() {
                    // Fixup the markdown to work better in the context of
                    // mkdocs-material
                    let chunk = chunk
                        .replace("###### **Options:**", "## Options")
                        .replace("###### **Arguments:**", "## Arguments")
                        .replace("\n  ", "\n    ")
                        .replace("\n*", "\n\n*");

                    if idx == 0 {
                        std::fs::write(
                            "docs/reference/kcli/_index.md",
                            format!(
                                "{chunk}\n\n## Available Subcommands {{ data-search-exclude }}"
                            ),
                        )?;
                    } else {
                        let (sub_command, remainder) = chunk.split_once('`').unwrap();
                        let filename = format!("docs/reference/kcli/{sub_command}.md");

                        let tags = match doc_tags.get(sub_command) {
                            Some(tags) => {
                                format!("---\ntags:\n  - {}\n---\n", tags.join("\n  - "))
                            }
                            None => String::new(),
                        };

                        let help = format!("{tags}# kcli {sub_command}\n{remainder}");
                        std::fs::write(&filename, &help)?;
                    }
                }

                Ok(())
            }
            Self::Bounce(cmd) => cmd.run(endpoint).await,
            Self::BounceCancel(cmd) => cmd.run(endpoint).await,
            Self::BounceList(cmd) => cmd.run(endpoint).await,
            Self::Rebind(cmd) => cmd.run(endpoint).await,
            Self::Suspend(cmd) => cmd.run(endpoint).await,
            Self::SuspendCancel(cmd) => cmd.run(endpoint).await,
            Self::SuspendList(cmd) => cmd.run(endpoint).await,
            Self::SuspendReadyQ(cmd) => cmd.run(endpoint).await,
            Self::SuspendReadyQCancel(cmd) => cmd.run(endpoint).await,
            Self::SuspendReadyQList(cmd) => cmd.run(endpoint).await,
            Self::SetLogFilter(cmd) => cmd.run(endpoint).await,
            Self::InspectMessage(cmd) => cmd.run(endpoint).await,
            Self::InspectSchedQ(cmd) => cmd.run(endpoint).await,
            Self::ProviderSummary(cmd) => cmd.run(endpoint).await,
            Self::QueueSummary(cmd) => cmd.run(endpoint).await,
            Self::TraceSmtpClient(cmd) => cmd.run(endpoint).await,
            Self::TraceSmtpServer(cmd) => cmd.run(endpoint).await,
            Self::Top(cmd) => cmd.run(endpoint).await,
            Self::Xfer(cmd) => cmd.run(endpoint).await,
            Self::XferCancel(cmd) => cmd.run(endpoint).await,
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

pub async fn request_with_streaming_text_response<T: reqwest::IntoUrl, B: serde::Serialize>(
    method: reqwest::Method,
    url: T,
    body: &B,
) -> anyhow::Result<impl Stream<Item = reqwest::Result<bytes::Bytes>>> {
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
        let body_text = String::from_utf8_lossy(&body_bytes);
        anyhow::bail!(
            "request status {}: {}. Response body: {body_text}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
        );
    }

    Ok(response.bytes_stream())
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

#[derive(ValueEnum, Default, Debug, Clone, Copy)]
pub enum ColorMode {
    #[default]
    Tty,
    Yes,
    No,
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
