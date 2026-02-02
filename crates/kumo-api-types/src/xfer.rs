use serde::{Deserialize, Serialize};
use url::Url;
use utoipa::{ToResponse, ToSchema};

#[derive(Deserialize, Serialize, Debug, Clone, ToSchema, PartialEq)]
pub struct XferProtocol {
    /// Expected to be an HTTP url prefix like:
    /// `https://host.name:8008`
    /// `http://127.0.0.1:8000`
    // TODO: support multiple, as well as resolving the hostname
    // to multiple candidates so that we can immediately retry
    // transient issues on subsequent candidates
    #[schema(examples("http://127.0.0.1:8000", "https://host.name:8008"))]
    pub target: Url,
}

const XFER_QUEUE_SUFFIX: &str = ".xfer.kumomta.internal";

impl XferProtocol {
    pub fn is_xfer_queue_name(name: &str) -> bool {
        name.ends_with(XFER_QUEUE_SUFFIX)
    }

    pub fn to_queue_name(&self) -> String {
        format!("{}{XFER_QUEUE_SUFFIX}", self.target)
    }

    pub fn from_queue_name(name: &str) -> Option<Self> {
        let name = name.strip_suffix(XFER_QUEUE_SUFFIX)?;
        let target: Url = name.parse().ok()?;
        Some(Self { target })
    }
}

/// Describes which messages should be transferred to another
/// kumomta node.
/// The criteria apply to the scheduled queue associated
/// with a given message.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct XferV1Request {
    /// The campaign name to match. If omitted, any campaign will match.
    #[serde(default)]
    pub campaign: Option<String>,

    /// The tenant to match. If omitted, any tenant will match.
    #[serde(default)]
    pub tenant: Option<String>,

    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Option<String>,

    /// The routing_domain name to match. If omitted, any routing_domain will match.
    #[serde(default)]
    pub routing_domain: Option<String>,

    /// Reason to log in the delivery log. Each matching message will log
    /// with an AdminRebind record to indicate that it was moved from
    /// its containing queue, and this reason will be included in that record.
    #[schema(example = "Scaling down")]
    pub reason: String,

    /// If present, queue_names takes precedence over `campaign`,
    /// `tenant`, and `domain` and specifies the exact set of
    /// scheduled queue names to which the xfer applies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue_names: Vec<String>,

    #[serde(flatten)]
    pub protocol: XferProtocol,
}

#[derive(Serialize, Deserialize, Debug, ToSchema, ToResponse)]
pub struct XferV1Response {}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct XferCancelV1Request {
    /// The name of the xfer scheduled queue
    pub queue_name: String,

    /// Reason to log in the delivery log. Each matching message will log
    /// with an AdminRebind record to indicate that it was moved from
    /// its containing queue, and this reason will be included in that record.
    #[schema(example = "Scaling down")]
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, ToSchema, ToResponse)]
pub struct XferCancelV1Response {}
