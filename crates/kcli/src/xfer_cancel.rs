use clap::Parser;
use kumo_api_types::xfer::{XferCancelV1Request, XferCancelV1Response};
use reqwest::Url;

#[derive(Debug, Parser)]
/// Cancels a message transfer that was initiated via the xfer
/// subcommand.  You specify the name of the xfer queue associated
/// with the transfer and matching messages will be taken out of
/// that queue and returned to their originating queue.
pub struct XferCancelCommand {
    /// The name of the xfer queue that you wish to cancel
    pub queue_name: String,

    /// Each matching message will be rebound into its originating
    /// queue, and an AdminRebind log will be generated to
    /// trace that the rebind happened.  The reason you specify
    /// here will be included in that log record.
    #[arg(long)]
    reason: String,
}

impl XferCancelCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let url = endpoint.join("/api/admin/xfer/cancel/v1")?;
        let request = XferCancelV1Request {
            queue_name: self.queue_name.clone(),
            reason: self.reason.clone(),
        };

        let result: XferCancelV1Response =
            crate::request_with_json_response(reqwest::Method::POST, url, &request).await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
