use crate::rfc5965::ARFReport;
use bounce_classify::BounceClass;
use chrono::{DateTime, Utc};
use rfc5321::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use uuid::Uuid;

pub mod rfc3464;
pub mod rfc5965;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedAddress {
    pub name: String,
    pub addr: IpAddr,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum RecordType {
    /// Recorded by a receiving listener
    Reception,
    /// Recorded by the delivery side, most likely as a
    /// result of attempting a delivery to a remote host
    Delivery,
    Bounce,
    TransientFailure,
    /// Recorded when a message is expiring from the queue
    Expiration,
    /// Administratively failed
    AdminBounce,
    /// Contains information about an OOB bounce
    OOB,
    /// Contains a feedback report
    Feedback,

    /// Special for matching anything in the logging config
    Any,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonLogRecord {
    /// What kind of record this is
    #[serde(rename = "type")]
    pub kind: RecordType,
    /// The message id
    pub id: String,
    /// The envelope sender
    pub sender: String,
    /// The envelope recipient
    pub recipient: String,
    /// Which named queue the message was associated with
    pub queue: String,
    /// Which MX site the message was being delivered to
    pub site: String,
    /// The size of the message, in bytes
    pub size: u64,
    /// The response from/to the peer
    pub response: Response,
    /// The address of the peer, and our sense of its
    /// hostname or EHLO domain
    pub peer_address: Option<ResolvedAddress>,
    /// The time at which we are logging this event
    #[serde(with = "chrono::serde::ts_seconds")]
    pub timestamp: DateTime<Utc>,
    /// The time at which the message was initially received and created
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created: DateTime<Utc>,
    /// The number of delivery attempts that have been made.
    /// Note that this may be approximate after a restart; use the
    /// number of logged events to determine the true number
    pub num_attempts: u16,

    pub bounce_classification: BounceClass,

    pub egress_pool: Option<String>,
    pub egress_source: Option<String>,

    pub feedback_report: Option<ARFReport>,

    pub meta: HashMap<String, Value>,
    pub headers: HashMap<String, Value>,

    /// The protocol used to deliver, or attempt to deliver, this message
    pub delivery_protocol: Option<String>,

    /// The protocol used to receive this message
    pub reception_protocol: Option<String>,

    /// The id of the node on which the event occurred
    pub nodeid: Uuid,

    /// The TLS Cipher used, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_cipher: Option<String>,

    /// The TLS protocol version used, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_protocol_version: Option<String>,

    /// The Subject Name from the peer TLS certificate, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_peer_subject_name: Option<Vec<String>>,
}
