use clap::{ArgGroup, Parser};
use kumo_api_client::KumoApiClient;
use kumo_api_types::xfer::{XferProtocol, XferV1Request};
use reqwest::Url;

#[derive(Debug, Parser)]
/// Transfer messages from matching queues to an alternative
/// kumomta node.
///
/// The intended purpose of this command is to facilitate manual
/// migration of queues to alternative nodes as part of planned
/// maintenance or part of an orchestrated down-scaling operation.
///
/// Xfering works first by selecting the set of scheduled queues
/// based on matching criteria that you specify via the `--domain`,
/// `--routing-domain`, `--campaign`, `--tenant`, `--queue`,
/// and/or `--everything` options.
///
/// Each matching queue has its messages drained and the xfer
/// logic will amend the message metadata to capture scheduling
/// and due time information and then place the message into
/// a special `.xfer.kumomta.internal` message transfer queue
/// where it will be immediately eligible to be moved to the
/// destination node.
///
/// Upon successful reception on the destination node, the saved
/// scheduling information will be restored to the message
/// and it will be inserted into an appropriate queue on
/// that destination node for delivery at the appropriate time.
///
/// Since the number of messages may be very large, and because
/// processing messages may result in a large amount of I/O
/// to load in every matching message's metadata, the total
/// amount of time taken for an xfer request may be too large
/// to feasibly wait for in the context of a simple request/response.
///
/// With that in mind, the xfer action runs asynchronously: aside from any
/// immediate syntax/request formatting issues, this command
/// will immediately return with no further status indication.
///
/// Errors will be reported in the diagnostic log.
///
/// ## Examples
///
/// Move messages from the "example.com" queue to the kumomta node
/// running an http listener on `http://10.0.0.1:8000`:
///
/// ```
///    kcli xfer --domain example.com --target http://10.0.0.1:8000
/// ```
#[clap(
    group(ArgGroup::new("selection")
        .multiple(true)
        .required(true)
        .args(&["domain", "routing_domain", "campaign", "tenant", "everything", "queue"])),
)]
pub struct XferCommand {
    /// The domain name to match.
    /// If omitted, any domains will match!
    #[arg(long)]
    domain: Option<String>,

    /// The routing_domain name to match.
    /// If omitted, any routing domain will match!
    #[arg(long)]
    routing_domain: Option<String>,

    /// The campaign name to match.
    /// If omitted, any campaigns will match!
    #[arg(long)]
    campaign: Option<String>,

    /// The tenant name to match.
    /// If omitted, any tenant will match!
    #[arg(long)]
    tenant: Option<String>,

    /// The precise name of a scheduled queue which should match.
    /// Can be specified multiple times.
    #[arg(long, conflicts_with_all=&["domain", "routing_domain", "campaign", "tenant"])]
    queue: Vec<String>,

    /// Each matching message will be rebound into an appropriate
    /// xfer queue, and an AdminRebind log will be generated to
    /// trace that the rebind happened.  The reason you specify
    /// here will be included in that log record.
    #[arg(long)]
    reason: String,

    /// Match all queues.
    #[arg(long, conflicts_with_all=&["domain", "routing_domain", "campaign", "tenant", "queue"])]
    everything: bool,

    /// Which node to transfer the messages to.
    /// This should be an HTTP URL prefix that will reach the
    /// HTTP listener on the target node, such as `http://hostname:8000`
    #[arg(long)]
    target: Url,
}

impl XferCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        if self.domain.is_none()
            && self.campaign.is_none()
            && self.tenant.is_none()
            && self.routing_domain.is_none()
            && self.queue.is_empty()
            && !self.everything
        {
            anyhow::bail!(
                "No domain, routing_domain, campaign or tenant was specified. \
                 Use --everything if you intend to apply to all queues"
            );
        }

        let client = KumoApiClient::new(endpoint.clone());
        let _result = client
            .admin_xfer_v1(&XferV1Request {
                campaign: self.campaign.clone(),
                domain: self.domain.clone(),
                routing_domain: self.routing_domain.clone(),
                tenant: self.tenant.clone(),
                reason: self.reason.clone(),
                queue_names: self.queue.clone(),
                protocol: XferProtocol {
                    target: self.target.clone(),
                },
            })
            .await?;

        eprintln!("NOTE: Xfer always runs asynchronously");

        Ok(())
    }
}
