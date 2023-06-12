use clap::Parser;
use kumo_api_types::SuspendReadyQueueV1ListEntry;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns list of current ready queue/egress path suspend rules.
///
/// Returns the list of un-expired admin suspends that are
/// currently in effect on the target instance.
pub struct SuspendReadyQListCommand {}

impl SuspendReadyQListCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let result: Vec<SuspendReadyQueueV1ListEntry> = crate::request_with_json_response(
            reqwest::Method::GET,
            endpoint.join("/api/admin/suspend-ready-q/v1")?,
            &(),
        )
        .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
