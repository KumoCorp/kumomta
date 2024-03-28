use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize, Default)]
pub struct Suspensions {
    pub ready_q: Vec<ReadyQSuspension>,
    pub sched_q: Vec<SchedQSuspension>,
}

#[derive(Serialize, Default, Clone)]
pub struct ReadyQSuspension {
    pub rule_hash: String,
    pub site_name: String,
    pub reason: String,
    pub source: String,
    pub expires: DateTime<Utc>,
}

#[derive(Serialize, Default, Clone)]
pub struct SchedQSuspension {
    pub rule_hash: String,
    pub tenant: String,
    pub domain: String,
    pub campaign: Option<String>,
    pub reason: String,
    pub expires: DateTime<Utc>,
}

#[derive(Serialize, Clone)]
pub enum SuspensionEntry {
    ReadyQ(ReadyQSuspension),
    SchedQ(SchedQSuspension),
}
