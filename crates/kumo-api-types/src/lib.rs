use chrono::{DateTime, Utc};
use cidr_map::CidrSet;
use serde::{Deserialize, Serialize};
use serde_with::formats::PreferOne;
use serde_with::{serde_as, OneOrMany};
use spool::SpoolId;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;
use utoipa::{IntoParams, ToResponse, ToSchema};
use uuid::Uuid;

pub mod egress_path;
pub mod rebind;
pub mod shaping;
pub mod tsa;
pub mod xfer;

/// Describes which messages should be bounced.
/// The criteria apply to the scheduled queue associated
/// with a given message.
///
/// !!! danger
///     If you specify none of `domain`, `campaign`, `tenant`,
///     `routing_domain` or `queue`, then **ALL** queues will
///     be bounced.
///
///     With great power comes great responsibility!
///
#[derive(Serialize, Deserialize, Debug, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BounceV1Request {
    /// The campaign name to match. If omitted, any campaign will match.
    #[serde(default)]
    #[schema(example = "campaign_name")]
    pub campaign: Option<String>,

    /// The tenant to match. If omitted, any tenant will match.
    #[serde(default)]
    #[schema(example = "tenant_name")]
    pub tenant: Option<String>,

    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Option<String>,

    /// The routing_domain name to match. If omitted, any routing_domain will match.
    /// {{since('2023.08.22-4d895015', inline=True)}}
    #[serde(default)]
    #[schema(example = "routing_domain.com")]
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
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(example = "20m")]
    pub duration: Option<Duration>,

    /// If true, do not generate AdminBounce delivery logs for matching
    /// messages.
    #[serde(default)]
    #[schema(default = false)]
    pub suppress_logging: bool,

    /// instead of specifying the duration, you can set an explicit
    /// expiration timestamp
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<Utc>>,

    /// If present, queue_names takes precedence over `campaign`,
    /// `tenant`, and `domain` and specifies the exact set of
    /// scheduled queue names to which the bounce applies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(example=json!(["campaign_name:tenant_name@example.com"]))]
    pub queue_names: Vec<String>,
}

impl BounceV1Request {
    pub fn duration(&self) -> Duration {
        match &self.expires {
            Some(exp) => (*exp - Utc::now()).to_std().unwrap_or(Duration::ZERO),
            None => self.duration.unwrap_or_else(default_duration),
        }
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
    /// Deprecated: this field is no longer populated, as bounces
    /// are now always asynchronous. In earlier versions the following
    /// applies:
    ///
    /// A map of queue name to number of bounced messages that
    /// were processed as part of the initial sweep.
    /// Additional bounces may be generated if/when other messages
    /// that match the rule are discovered, but those obviously
    /// cannot be reported in the context of the initial request.
    #[schema(deprecated, example=json!({
        "gmail.com": 200,
        "yahoo.com": 100
    }))]
    pub bounced: HashMap<String, usize>,
    /// Deprecated: this field is no longer populated, as bounces are
    /// now always asynchronous. In earlier versions the following applies:
    ///
    /// The sum of the number of bounced messages reported by
    /// the `bounced` field.
    #[schema(deprecated, example = 300)]
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
    #[schema(example = "campaign_name")]
    pub campaign: Option<String>,
    /// The tenant field of the original request, if any.
    #[serde(default)]
    #[schema(example = "tenant_name")]
    pub tenant: Option<String>,
    /// The domain field of the original request, if any.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Option<String>,
    /// The routing_domain field of the original request, if any.
    #[serde(default)]
    #[schema(example = "routing_domain.com")]
    pub routing_domain: Option<String>,

    /// The reason field of the original request
    #[schema(example = "cleaning up a bad send")]
    pub reason: String,

    /// The time remaining until this entry expires and is automatically
    /// removed.
    #[serde(with = "duration_serde")]
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
    #[schema(example = "campaign_name")]
    pub campaign: Option<String>,
    /// The tenant name to match. If omitted, any tenant will match.
    #[serde(default)]
    #[schema(example = "tenant_name")]
    pub tenant: Option<String>,
    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Option<String>,

    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a block with the destination postmaster")]
    pub reason: String,

    /// Specifies how long this suspension remains active.
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<Duration>,

    /// instead of specifying the duration, you can set an explicit
    /// expiration timestamp
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<Utc>>,

    /// If present, queue_names takes precedence over `campaign`,
    /// `tenant`, and `domain` and specifies the exact set of
    /// scheduled queue names to which the suspension applies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(example=json!(["campaign_name:tenant_name@example.com"]))]
    pub queue_names: Vec<String>,
}

impl SuspendV1Request {
    pub fn duration(&self) -> Duration {
        match &self.expires {
            Some(exp) => (*exp - Utc::now()).to_std().unwrap_or(Duration::ZERO),
            None => self.duration.unwrap_or_else(default_duration),
        }
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
    #[schema(example = "campaign_name")]
    pub campaign: Option<String>,
    /// The tenant name to match. If omitted, any tenant will match.
    #[serde(default)]
    #[schema(example = "tenant_name")]
    pub tenant: Option<String>,
    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Option<String>,

    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a deliverability issue")]
    pub reason: String,

    #[serde(with = "duration_serde")]
    /// Specifies how long this suspension remains active.
    pub duration: Duration,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendReadyQueueV1Request {
    /// The name of the ready queue that should be suspended
    #[schema(
        example = "source_name->(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com@smtp_client"
    )]
    pub name: String,
    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a block with the destination postmaster")]
    pub reason: String,
    /// Specifies how long this suspension remains active.
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<Utc>>,
}

impl SuspendReadyQueueV1Request {
    pub fn duration(&self) -> Duration {
        if let Some(expires) = &self.expires {
            let duration = expires.signed_duration_since(Utc::now());
            duration.to_std().unwrap_or(Duration::ZERO)
        } else {
            self.duration.unwrap_or_else(default_duration)
        }
    }
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct SuspendReadyQueueV1ListEntry {
    /// The id for the suspension. Can be used to cancel the suspension.
    pub id: Uuid,
    /// The name of the ready queue that is suspended
    #[schema(
        example = "source_name->(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com@smtp_client"
    )]
    pub name: String,
    /// The reason for the suspension
    #[schema(example = "pause while working on resolving a block with the destination postmaster")]
    pub reason: String,

    /// how long until this suspension expires and is automatically removed
    #[serde(with = "duration_serde")]
    pub duration: Duration,

    /// The time at which the suspension will expire
    pub expires: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, IntoParams, ToSchema)]
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

#[derive(Serialize, Deserialize, Debug, IntoParams, ToSchema)]
pub struct InspectQueueV1Request {
    /// The name of the scheduled queue
    #[schema(example = "campaign_name:tenant_name@example.com")]
    pub queue_name: String,
    /// If true, return the message body in addition to the
    /// metadata
    #[serde(default)]
    pub want_body: bool,

    /// Return up to `limit` messages in the queue sample.
    /// Depending on the strategy configured for the queue,
    /// messages may not be directly reachable via this endpoint.
    /// If no limit is provided, all messages in the queue will
    /// be sampled.
    #[serde(default)]
    pub limit: Option<usize>,
}

impl InspectQueueV1Request {
    pub fn apply_to_url(&self, url: &mut Url) {
        let mut query = url.query_pairs_mut();
        query.append_pair("queue_name", &self.queue_name.to_string());
        if self.want_body {
            query.append_pair("want_body", "true");
        }
        if let Some(limit) = self.limit {
            query.append_pair("limit", &limit.to_string());
        }
    }
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct InspectQueueV1Response {
    #[schema(example = "campaign_name:tenant_name@example.com")]
    pub queue_name: String,
    pub messages: Vec<InspectMessageV1Response>,
    pub num_scheduled: usize,
    #[schema(value_type=Object)]
    pub queue_config: serde_json::Value,
    pub delayed_metric: usize,
    pub now: DateTime<Utc>,
    pub last_changed: DateTime<Utc>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct MessageInformation {
    /// The envelope sender
    #[schema(example = "sender@sender.example.com")]
    pub sender: String,
    /// The envelope-to address.
    /// May be either an individual string or an array of strings
    /// for multi-recipient messages.
    #[schema(example = "recipient@example.com", format = "email")]
    #[serde_as(as = "OneOrMany<_, PreferOne>")] // FIXME: json schema
    pub recipient: Vec<String>,
    /// The message metadata
    #[schema(value_type=Object, example=json!({
        "received_from": "10.0.0.1:3488"
    }))]
    pub meta: serde_json::Value,
    /// If `want_body` was set in the original request,
    /// holds the message body
    #[serde(default)]
    #[schema(example = "From: user@example.com\nSubject: Hello\n\nHello there")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_attempts: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type=Object)]
    pub scheduling: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct TraceSmtpV1Request {
    #[serde(default)]
    #[schema(value_type=Option<Vec<String>>)]
    pub source_addr: Option<CidrSet>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub terse: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
pub struct TraceSmtpV1Event {
    pub conn_meta: serde_json::Value,
    pub payload: TraceSmtpV1Payload,
    pub when: DateTime<Utc>,
}

#[serde_as]
#[derive(Clone, Serialize, Deserialize, Debug, ToSchema, PartialEq)]
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
        log_arf: serde_json::Value,
        log_oob: serde_json::Value,
        queue: String,
        meta: serde_json::Value,
        #[schema(format = "email")]
        sender: String,
        #[serde_as(as = "OneOrMany<_, PreferOne>")] // FIXME: json schema
        #[schema(format = "email")]
        recipient: Vec<String>,
        id: SpoolId,
        #[serde(default)]
        was_arf_or_oob: Option<bool>,
        #[serde(default)]
        will_enqueue: Option<bool>,
    },
    /// Like `Read`, but abbreviated by `terse`
    AbbreviatedRead {
        /// The "first" or more relevant line(s)
        snippet: String,
        /// Total size of data being read
        len: usize,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
pub struct TraceSmtpClientV1Event {
    pub conn_meta: serde_json::Value,
    pub payload: TraceSmtpClientV1Payload,
    pub when: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema, PartialEq)]
pub enum TraceSmtpClientV1Payload {
    BeginSession,
    Connected,
    Closed,
    Read(String),
    Write(String),
    Diagnostic {
        level: String,
        message: String,
    },
    MessageObtained,
    /// Like `Write`, but abbreviated by `terse`
    AbbreviatedWrite {
        /// The "first" or more relevant line(s)
        snippet: String,
        /// Total size of data being read
        len: usize,
    },
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct TraceSmtpClientV1Request {
    /// The campaign name to match. If omitted, any campaign will match.
    #[serde(default)]
    #[schema(example = "campaign_name")]
    pub campaign: Vec<String>,

    /// The tenant to match. If omitted, any tenant will match.
    #[serde(default)]
    #[schema(example = "tenant_name")]
    pub tenant: Vec<String>,

    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Vec<String>,

    /// The routing_domain name to match. If omitted, any routing_domain will match.
    #[serde(default)]
    #[schema(example = "routing_domain.com")]
    pub routing_domain: Vec<String>,

    /// The egress pool name to match. If omitted, any egress pool will match.
    #[serde(default)]
    #[schema(example = "pool_name")]
    pub egress_pool: Vec<String>,

    /// The egress source name to match. If omitted, any egress source will match.
    #[serde(default)]
    #[schema(example = "source_name")]
    pub egress_source: Vec<String>,

    /// The envelope sender to match. If omitted, any will match.
    #[serde(default)]
    #[schema(format = "email")]
    pub mail_from: Vec<String>,

    /// The envelope recipient to match. If omitted, any will match.
    #[serde(default)]
    #[schema(format = "email")]
    pub rcpt_to: Vec<String>,

    /// The source address to match. If omitted, any will match.
    #[serde(default)]
    #[schema(value_type=Option<Vec<String>>, example="10.0.0.1/16")]
    pub source_addr: Option<CidrSet>,

    /// The mx hostname to match. If omitted, any will match.
    #[serde(default)]
    #[schema(format = "mx1.example.com")]
    pub mx_host: Vec<String>,

    /// The ready queue name to match. If omitted, any will match.
    #[serde(default)]
    #[schema(
        example = "source_name->(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com@smtp_client"
    )]
    pub ready_queue: Vec<String>,

    /// The mx ip address to match. If omitted, any will match.
    #[serde(default)]
    #[schema(value_type=Option<Vec<String>>, example="10.0.0.1/16")]
    pub mx_addr: Option<CidrSet>,

    /// Use a more terse representation of the data, focusing on the first
    /// line of larger writes
    #[serde(default, skip_serializing_if = "is_false")]
    pub terse: bool,
}

#[derive(Serialize, Deserialize, Debug, ToSchema, IntoParams)]
pub struct ReadyQueueStateRequest {
    /// Which queues to request. If empty, request all queue states.
    #[serde(default)]
    #[schema(example=json!(["campaign_name:tenant_name@example.com"]))]
    pub queues: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct QueueState {
    #[schema(example = "TooManyLeases for queue")]
    pub context: String,
    pub since: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct ReadyQueueStateResponse {
    pub states_by_ready_queue: HashMap<String, HashMap<String, QueueState>>,
}
