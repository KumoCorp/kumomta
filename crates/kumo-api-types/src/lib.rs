use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

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
