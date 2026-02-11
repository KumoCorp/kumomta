use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::BounceV1CancelRequest;
use reqwest::Url;
use uuid::Uuid;

#[derive(Debug, Parser)]
/// Cancels an admin bounce entry.
///
/// Cancelling the entry prevents it from matching new messages.
/// It cannot retroactively un-bounce messages that it already
/// matched and bounced.
pub struct BounceCancelCommand {
    /// The id field of the bounce entry that you wish to cancel
    #[arg(long, value_parser=Uuid::parse_str)]
    pub id: Uuid,
}

impl BounceCancelCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let response = client
            .admin_bounce_cancel_v1(&BounceV1CancelRequest { id: self.id })
            .await?;

        if !response.is_empty() {
            println!("{response}");
        } else {
            println!("OK");
        }

        Ok(())
    }
}
