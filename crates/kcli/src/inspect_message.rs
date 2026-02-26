use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::InspectMessageV1Request;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns information about a message in the spool
pub struct InspectMessageCommand {
    #[arg(long)]
    pub want_body: bool,

    pub id: String,
}

impl InspectMessageCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let result = client
            .admin_inspect_message_v1(&InspectMessageV1Request {
                id: self.id.clone().try_into()?,
                want_body: self.want_body,
            })
            .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
