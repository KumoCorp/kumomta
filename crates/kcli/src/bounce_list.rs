use clap::Parser;
use kumo_api_types::BounceV1ListEntry;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns list of current administrative bounce rules.
///
/// Returns the list of un-expired admin bounces that are
/// currently in effect on the target instance.
pub struct BounceListCommand {}

impl BounceListCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let result: Vec<BounceV1ListEntry> = reqwest::get(endpoint.join("/api/admin/bounce/v1")?)
            .await?
            .json()
            .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
