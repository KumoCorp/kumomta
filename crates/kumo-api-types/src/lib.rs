use chrono::{DateTime, Utc};
use cidr_map::CidrSet;
use serde::{Deserialize, Serialize};
use spool::SpoolId;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;
use utoipa::{IntoParams, ToResponse, ToSchema};
use uuid::Uuid;

pub mod egress_path;
pub mod shaping;

/// Describes which messages should be bounced.
/// The criteria apply to the scheduled queue associated
/// with a given message.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct BounceV1Request {
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

    /// Reason to log in the delivery log. Each matching message will be bounced
    /// with an AdminBounce record unless you suppress logging.
    /// The reason will also be shown in the list of currently active admin
    /// bounces.
    #[schema(example = "Cleaning up a bad send")]
    pub reason: String,

    /// Defaults to "5m". Specifies how long this bounce directive remains active.
    /// While active, newly injected messages that match the bounce criteria
    /// will also be bounced.
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(example = "20m")]
    pub duration: Option<Duration>,

    /// If true, do not generate AdminBounce delivery logs for matching
    /// messages.
    #[serde(default)]
    pub suppress_logging: bool,
}

impl BounceV1Request {
    pub fn duration(&self) -> Duration {
        self.duration.unwrap_or_else(default_duration)
    }
}

fn default_duration() -> Duration {
    Duration::from_secs(300)
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct BounceV1Response {
    /// The id of the bounce rule that was registered.
    /// This can be used later to delete the rule if desired.
    #[schema(example = "552016f1-08e7-4e90-9da3-fd5c25acd069")]
    pub id: Uuid,
    /// A map of queue name to number of bounced messages that
    /// were processed as part of the initial sweep.
    /// Additional bounces may be generated if/when other messages
    /// that match the rule are discovered, but those obviously
    /// cannot be reported in the context of the initial request.
    #[schema(example=json!({
        "gmail.com": 200,
        "yahoo.com": 100
    }))]
    pub bounced: HashMap<String, usize>,
    /// The sum of the number of bounced messages reported by
    /// the `bounced` field.
    #[schema(example = 300)]
    pub total_bounced: usize,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SetDiagnosticFilterRequest {
    /// The diagnostic filter spec to use
    #[schema(example = "kumod=trace")]
    pub filter: String,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct BounceV1ListEntry {
    /// The id of this bounce rule. Corresponds to the `id` field
    /// returned by the originating request that set up the bounce,
    /// and can be used to identify this particular entry if you
    /// wish to delete it later.
    #[schema(example = "552016f1-08e7-4e90-9da3-fd5c25acd069")]
    pub id: Uuid,

    /// The campaign field of the original request, if any.
    #[serde(default)]
    pub campaign: Option<String>,
    /// The tenant field of the original request, if any.
    #[serde(default)]
    pub tenant: Option<String>,
    /// The domain field of the original request, if any.
    #[serde(default)]
    pub domain: Option<String>,
    /// The routing_domain field of the original request, if any.
    #[serde(default)]
    pub routing_domain: Option<String>,

    /// The reason field of the original request
    pub reason: String,

    /// The time remaining until this entry expires and is automatically
    /// removed.
    #[serde(with = "humantime_serde")]
    pub duration: Duration,

    /// A map of queue name to number of bounced messages that
    /// were processed by this entry since it was created.
    #[schema(example=json!({
        "gmail.com": 200,
        "yahoo.com": 100
    }))]
    pub bounced: HashMap<String, usize>,
    /// The sum of the number of bounced messages reported by
    /// the `bounced` field.
    pub total_bounced: usize,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct BounceV1CancelRequest {
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendV1Request {
    /// The campaign name to match. If omitted, any campaign will match.
    #[serde(default)]
    pub campaign: Option<String>,
    /// The tenant name to match. If omitted, any tenant will match.
    #[serde(default)]
    pub tenant: Option<String>,
    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    pub domain: Option<String>,

    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a block with the destination postmaster")]
    pub reason: String,

    /// Specifies how long this suspension remains active.
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<Duration>,
}

impl SuspendV1Request {
    pub fn duration(&self) -> Duration {
        self.duration.unwrap_or_else(default_duration)
    }
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct SuspendV1Response {
    /// The id of the suspension. This can be used later to cancel
    /// the suspension.
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendV1CancelRequest {
    /// The id of the suspension to cancel
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendV1ListEntry {
    /// The id of the suspension. This can be used later to cancel
    /// the suspension.
    pub id: Uuid,

    /// The campaign name to match. If omitted, any campaign will match.
    #[serde(default)]
    pub campaign: Option<String>,
    /// The tenant name to match. If omitted, any tenant will match.
    #[serde(default)]
    pub tenant: Option<String>,
    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    pub domain: Option<String>,

    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a deliverability issue")]
    pub reason: String,

    #[serde(with = "humantime_serde")]
    /// Specifies how long this suspension remains active.
    pub duration: Duration,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendReadyQueueV1Request {
    /// The name of the ready queue that should be suspended
    pub name: String,
    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a block with the destination postmaster")]
    pub reason: String,
    /// Specifies how long this suspension remains active.
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<Duration>,
}

impl SuspendReadyQueueV1Request {
    pub fn duration(&self) -> Duration {
        self.duration.unwrap_or_else(default_duration)
    }
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendReadyQueueV1ListEntry {
    /// The id for the suspension. Can be used to cancel the suspension.
    pub id: Uuid,
    /// The name of the ready queue that is suspended
    pub name: String,
    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a block with the destination postmaster")]
    pub reason: String,

    /// how long until this suspension expires and is automatically removed
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
}

#[derive(Serialize, Deserialize, Debug, IntoParams)]
pub struct InspectMessageV1Request {
    /// The spool identifier for the message whose information
    /// is being requested
    pub id: SpoolId,
    /// If true, return the message body in addition to the
    /// metadata
    #[serde(default)]
    pub want_body: bool,
}

impl InspectMessageV1Request {
    pub fn apply_to_url(&self, url: &mut Url) {
        let mut query = url.query_pairs_mut();
        query.append_pair("id", &self.id.to_string());
        if self.want_body {
            query.append_pair("want_body", "true");
        }
    }
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct InspectMessageV1Response {
    /// The spool identifier of the message
    pub id: SpoolId,
    /// The message information
    pub message: MessageInformation,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct MessageInformation {
    /// The envelope sender
    #[schema(example = "sender@sender.example.com")]
    pub sender: String,
    /// The envelope-to address
    #[schema(example = "recipient@example.com")]
    pub recipient: String,
    /// The message metadata
    #[schema(example=json!({
        "received_from": "10.0.0.1:3488"
    }))]
    pub meta: serde_json::Value,
    /// If `want_body` was set in the original request,
    /// holds the message body
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct TraceSmtpV1Request {
    #[serde(default)]
    pub source_addr: Option<CidrSet>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
pub struct TraceSmtpV1Event {
    pub conn_meta: serde_json::Value,
    pub payload: TraceSmtpV1Payload,
    pub when: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
pub enum TraceSmtpV1Payload {
    Connected,
    Closed,
    Read(String),
    Write(String),
    Diagnostic {
        level: String,
        message: String,
    },
    Callback {
        name: String,
        result: Option<serde_json::Value>,
        error: Option<String>,
    },
    MessageDisposition {
        relay: bool,
        log_arf: bool,
        log_oob: bool,
        queue: String,
        meta: serde_json::Value,
        sender: String,
        recipient: String,
        id: SpoolId,
    },
}
