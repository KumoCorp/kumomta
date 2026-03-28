use clap::{ArgGroup, Parser};
use kumo_api_client::KumoApiClient;
use kumo_api_types::SuspendV1Request;
use reqwest::Url;
use std::time::Duration;

#[derive(Debug, Parser)]
/// Administratively suspend messages in matching queues.
#[clap(
    group(ArgGroup::new("selection")
        .multiple(true)
        .required(true)
        .args(&["domain", "campaign", "tenant", "everything", "queue"])),
)]
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
    #[arg(long, conflicts_with_all=&["domain", "campaign", "tenant", "queue"])]
    everything: bool,

    /// Suspend specific scheduled queue names using their exact queue name(s).
    /// Can be specified multiple times.
    #[arg(long, conflicts_with_all=&["domain", "campaign", "tenant"])]
    queue: Vec<String>,

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
            && self.queue.is_empty()
            && !self.everything
        {
            anyhow::bail!(
                "No domain, campaign or tenant was specified. \
                 Use --everything if you intend to suspend all queues"
            );
        }

        let client = KumoApiClient::new(endpoint.clone());
        let result = client
            .admin_suspend_v1(&SuspendV1Request {
                campaign: self.campaign.clone(),
                domain: self.domain.clone(),
                tenant: self.tenant.clone(),
                reason: self.reason.clone(),
                duration: self.duration,
                expires: None,
                queue_names: self.queue.clone(),
            })
            .await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
