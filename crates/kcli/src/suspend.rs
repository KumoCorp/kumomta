use clap::Parser;
use kumo_api_types::{SuspendV1Request, SuspendV1Response};
use reqwest::Url;
use std::time::Duration;

#[derive(Debug, Parser)]
/// Administratively suspend messages in matching queues.
pub struct SuspendCommand {
    /// The domain name to match.
    /// If omitted, any domains will match!
    #[arg(long)]
    domain: Option<String>,

    /// The campaign name to match.
    /// If omitted, any campaigns will match!
    #[arg(long)]
    campaign: Option<String>,

    /// The tenant name to match.
    /// If omitted, any tenant will match!
    #[arg(long)]
    tenant: Option<String>,

    /// The reason to log in the delivery logs
    #[arg(long)]
    reason: String,

    /// Suspend all queues.
    #[arg(long)]
    everything: bool,

    /// The duration over which matching messages will continue to suspend.
    /// The default is '5m'.
    #[arg(long, value_parser=humantime::parse_duration)]
    duration: Option<Duration>,
}

impl SuspendCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        if self.domain.is_none()
            && self.campaign.is_none()
            && self.tenant.is_none()
            && !self.everything
        {
            anyhow::bail!(
                "No domain, campaign or tenant was specified. \
                 Use --everything if you intend to suspend all queues"
            );
        }

        let result: SuspendV1Response = crate::request_with_json_response(
            reqwest::Method::POST,
            endpoint.join("/api/admin/suspend/v1")?,
            &SuspendV1Request {
                campaign: self.campaign.clone(),
                domain: self.domain.clone(),
                tenant: self.tenant.clone(),
                reason: self.reason.clone(),
                duration: self.duration,
                expires: None,
            },
        )
        .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
