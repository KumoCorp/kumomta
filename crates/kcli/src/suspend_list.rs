use clap::Parser;
use kumo_api_types::SuspendV1ListEntry;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns list of current administrative suspend rules.
///
/// Returns the list of un-expired admin suspends that are
/// currently in effect on the target instance.
pub struct SuspendListCommand {}

impl SuspendListCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let result: Vec<SuspendV1ListEntry> = crate::request_with_json_response(
            reqwest::Method::GET,
            endpoint.join("/api/admin/suspend/v1")?,
            &(),
        )
        .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
