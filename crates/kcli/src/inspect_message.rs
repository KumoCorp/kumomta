use clap::Parser;
use kumo_api_types::{InspectMessageV1Request, InspectMessageV1Response};
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
        let mut url = endpoint.join("/api/admin/inspect-message/v1")?;
        let request = InspectMessageV1Request {
            id: self.id.clone().try_into()?,
            want_body: self.want_body,
        };
        request.apply_to_url(&mut url);

        let result: InspectMessageV1Response =
            crate::request_with_json_response(reqwest::Method::GET, url, &()).await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
