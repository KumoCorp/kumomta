use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize, Default)]
pub struct Suspensions {
    pub ready_q: Vec<ReadyQSuspension>,
}

#[derive(Serialize, Default, Clone)]
pub struct ReadyQSuspension {
    pub rule_hash: String,
    pub site_name: String,
    pub reason: String,
    pub source: String,
    pub expires: DateTime<Utc>,
}

#[derive(Serialize, Clone)]
pub enum SuspensionEntry {
    ReadyQ(ReadyQSuspension),
}
