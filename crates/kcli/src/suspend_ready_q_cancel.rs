use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::SuspendV1CancelRequest;
use reqwest::Url;
use uuid::Uuid;

#[derive(Debug, Parser)]
/// Cancels an admin suspend entry for a ready queue/egress path.
///
/// Cancelling the entry re-enables delivery via the specified
/// egress path.
pub struct SuspendReadyQCancelCommand {
    /// The id field of the suspend entry that you wish to cancel
    #[arg(long, value_parser=Uuid::parse_str)]
    pub id: Uuid,
}

impl SuspendReadyQCancelCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let response = client
            .admin_suspend_ready_q_cancel_v1(&SuspendV1CancelRequest { id: self.id })
            .await?;

        if !response.is_empty() {
            println!("{response}");
        } else {
            println!("OK");
        }

        Ok(())
    }
}
