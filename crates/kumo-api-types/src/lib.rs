use serde::{Deserialize, Serialize};
use spool::SpoolId;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

pub mod egress_path;
pub mod shaping;

#[derive(Serialize, Deserialize, Debug)]
pub struct BounceV1Request {
    #[serde(default)]
    pub campaign: Option<String>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,

    pub reason: String,
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<Duration>,
}

impl BounceV1Request {
    pub fn duration(&self) -> Duration {
        self.duration.unwrap_or_else(default_duration)
    }
}

fn default_duration() -> Duration {
    Duration::from_secs(300)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BounceV1Response {
    pub id: Uuid,
    pub bounced: HashMap<String, usize>,
    pub total_bounced: usize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetDiagnosticFilterRequest {
    pub filter: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BounceV1ListEntry {
    pub id: Uuid,

    #[serde(default)]
    pub campaign: Option<String>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,

    pub reason: String,

    #[serde(with = "humantime_serde")]
    pub duration: Duration,

    pub bounced: HashMap<String, usize>,
    pub total_bounced: usize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BounceV1CancelRequest {
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SuspendV1Request {
    #[serde(default)]
    pub campaign: Option<String>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,

    pub reason: String,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct SuspendV1Response {
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SuspendV1CancelRequest {
    pub id: Uuid,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SuspendV1ListEntry {
    pub id: Uuid,

    #[serde(default)]
    pub campaign: Option<String>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,

    pub reason: String,

    #[serde(with = "humantime_serde")]
    pub duration: Duration,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SuspendReadyQueueV1Request {
    pub name: String,
    pub reason: String,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct SuspendReadyQueueV1ListEntry {
    pub id: Uuid,
    pub name: String,
    pub reason: String,

    #[serde(with = "humantime_serde")]
    pub duration: Duration,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InspectMessageV1Request {
    pub id: SpoolId,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct InspectMessageV1Response {
    pub id: SpoolId,
    pub message: MessageInformation,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageInformation {
    pub sender: String,
    pub recipient: String,
    pub meta: serde_json::Value,
    #[serde(default)]
    pub data: Option<String>,
}
