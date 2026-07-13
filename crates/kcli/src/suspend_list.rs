use clap::Parser;
use kumo_api_client::KumoApiClient;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns list of current administrative suspend rules.
///
/// Returns the list of un-expired admin suspends that are
/// currently in effect on the target instance.
pub struct SuspendListCommand {}

impl SuspendListCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let result = client.admin_suspend_list_v1().await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
