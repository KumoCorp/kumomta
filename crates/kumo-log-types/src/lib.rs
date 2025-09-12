use crate::rfc5965::ARFReport;
use bounce_classify::BounceClass;
use chrono::{DateTime, Utc};
use kumo_address::host_or_socket::HostOrSocketAddress;
use kumo_address::socket::SocketAddress;
use rfc5321::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

pub mod rfc3464;
pub mod rfc5965;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedAddress {
    pub name: String,
    pub addr: HostOrSocketAddress,
}

impl std::fmt::Display for ResolvedAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let addr = format!("{}", self.addr);
        if addr == self.name {
            // likely: unix domain socket path
            write!(fmt, "{addr}")
        } else {
            write!(fmt, "{}/{addr}", self.name)
        }
    }
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

    /// SMTP Listener responded with a 4xx or 5xx
    Rejection,

    /// Administratively rebound from one queue to another
    AdminRebind,

    /// Moved from the special deferred injection queue
    /// and into some other queue
    DeferredInjectionRebind,

    /// Explains why a message was put into the scheduled queue
    Delayed,

    /// Special for matching anything in the logging config
    Any,
}

impl RecordType {
    /// Returns true if it makes sense to run the corresponding record
    /// through the bounce classifier module.
    /// The rule of thumb for that is if the response came from the
    /// destination when attempting delivery, but we also include
    /// administrative bounces and message expirations.
    pub const fn is_bounce_classifiable(&self) -> bool {
        match self {
            Self::Any
            | Self::Reception
            | Self::Delivery
            | Self::DeferredInjectionRebind
            | Self::AdminRebind
            | Self::Delayed => false,
            Self::Bounce
            | Self::TransientFailure
            | Self::Expiration
            | Self::AdminBounce
            | Self::OOB
            | Self::Feedback
            | Self::Rejection => true,
        }
    }
}

/// Unfortunately, when we defined the `timestamp` and `created` fields
/// in the log structure, we made the decision to log as the integer
/// unix timestamp format, which causes us to discard the sub-second
/// information that we otherwise have available.
///
/// We'd like to now include the full info in the serialized log
/// record, without bloating the in-memory representation, or otherwise
/// explicitly duplicating data to arrange for serde to emit it for us.
///
/// That's where this macro comes in; it allows us to serialize those
/// fields via a proxy type that effectively causes serde to emit two
/// different serializations of the same value.
///
/// Usage is: `ts_serializer(MODULE_NAME, STRUCT_NAME, SECONDS_FIELD, FULL_FIELD)`
///
/// The MODULE_NAME and STRUCT_NAME are not especially important and
/// are really present just for namespacing.
///
/// The SECONDS_FIELD defines the name of the field to be emitted
/// as the unix timestamp (in seconds).
///
/// The FULL_FIELD defines the name of the field to be emitted
/// as the full RFC 3339 datetime.
///
/// The macro defines a module and struct that can be used as a proxy
/// for serialization.
///
/// To actually use it, you need to annotate the field in the struct;
///
/// ```norun
/// #[serde(flatten, with = "ts_serializer")]
/// pub timestamp: DateTime<Utc>,
/// ```
///
/// It is important that `flatten` is used to avoid serde emitting
/// a nested/child struct, and the `with` attribute is what points
/// the serialization to the defined proxy module; it must
/// reference the MODULE_NAME you defined.
macro_rules! ts_serializer {
    ($module:ident, $name:ident, $seconds:ident, $full:ident) => {
        mod $module {
            use super::*;
            use serde::{Deserializer, Serializer};

            #[derive(Serialize, Deserialize, Copy, Clone, Eq, Hash, Default, Debug, PartialEq)]
            struct $name {
                #[serde(with = "chrono::serde::ts_seconds")]
                pub $seconds: DateTime<Utc>,

                /// Optional for backwards compatibility: we don't
                /// expect $full to be present, but we'll take it
                /// if it is!
                #[serde(default)]
                pub $full: Option<DateTime<Utc>>,
            }

            impl std::ops::Deref for $name {
                type Target = DateTime<Utc>;
                fn deref(&self) -> &DateTime<Utc> {
                    self.$full.as_ref().unwrap_or(&self.$seconds)
                }
            }

            impl<T: chrono::TimeZone> From<DateTime<T>> for $name
            where
                DateTime<Utc>: From<DateTime<T>>,
            {
                fn from(value: DateTime<T>) -> $name {
                    let timestamp: DateTime<Utc> = value.into();
                    $name {
                        $seconds: timestamp,
                        $full: Some(timestamp),
                    }
                }
            }

            impl From<$name> for DateTime<Utc> {
                fn from(value: $name) -> DateTime<Utc> {
                    *value
                }
            }

            pub fn serialize<S>(d: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let proxy: $name = (*d).into();
                proxy.serialize(s)
            }

            pub fn deserialize<'a, D>(d: D) -> Result<DateTime<Utc>, D::Error>
            where
                D: Deserializer<'a>,
            {
                $name::deserialize(d).map(|p| p.into())
            }
        }
    };
}

ts_serializer!(ts_serializer, TimestampSerializer, timestamp, event_time);
ts_serializer!(ct_serializer, CreationTimeSerializer, created, created_time);

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
    #[serde(flatten, with = "ts_serializer")]
    pub timestamp: DateTime<Utc>,
    /// The time at which the message was initially received and created
    #[serde(flatten, with = "ct_serializer")]
    pub created: DateTime<Utc>,
    /// The number of delivery attempts that have been made.
    /// Note that this may be approximate after a restart; use the
    /// number of logged events to determine the true number
    pub num_attempts: u16,

    pub bounce_classification: BounceClass,

    pub egress_pool: Option<String>,
    pub egress_source: Option<String>,
    pub source_address: Option<MaybeProxiedSourceAddress>,

    pub feedback_report: Option<Box<ARFReport>>,

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

    /// The provider name, if any.
    /// This is a way of grouping destination sites operated
    /// by the same provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,

    /// Uuid identifying a connection/session for either inbound
    /// or outbound (depending on the type of the record).
    /// This is useful when correlating a series of messages to
    /// the same connection for either ingress or egress
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaybeProxiedSourceAddress {
    pub address: SocketAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<SocketAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Cow<'static, str>>,
}

#[cfg(all(test, target_pointer_width = "64"))]
#[test]
fn sizes() {
    assert_eq!(std::mem::size_of::<JsonLogRecord>(), 712);
}
