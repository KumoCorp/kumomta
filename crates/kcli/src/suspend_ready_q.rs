use clap::Parser;
use kumo_api_types::{SuspendReadyQueueV1Request, SuspendV1Response};
use reqwest::Url;
use std::time::Duration;

#[derive(Debug, Parser)]
/// Administratively suspend the ready queue for an egress path
pub struct SuspendReadyQCommand {
    /// The name of the ready queue that you wish to suspend
    #[arg(long)]
    name: String,

    /// The reason to log in the delivery logs
    #[arg(long)]
    reason: String,

    /// The duration to suspend.
    /// The default is '5m'.
    #[arg(long, value_parser=humantime::parse_duration)]
    duration: Option<Duration>,
}

impl SuspendReadyQCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let result: SuspendV1Response = crate::request_with_json_response(
            reqwest::Method::POST,
            endpoint.join("/api/admin/suspend-ready-q/v1")?,
            &SuspendReadyQueueV1Request {
                name: self.name.clone(),
                reason: self.reason.clone(),
                duration: self.duration.clone(),
                expires: None,
            },
        )
        .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
