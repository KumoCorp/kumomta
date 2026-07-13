use clap::Parser;
use kumo_api_client::KumoApiClient;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns list of current ready queue/egress path suspend rules.
///
/// Returns the list of un-expired admin suspends that are
/// currently in effect on the target instance.
pub struct SuspendReadyQListCommand {}

impl SuspendReadyQListCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let result = client.admin_suspend_ready_q_list_v1().await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
