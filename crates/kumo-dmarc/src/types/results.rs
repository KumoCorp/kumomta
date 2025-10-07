use crate::types::identifier::Identifier;
use crate::types::policy::Policy;
use crate::types::policy_override::PolicyOverrideReason;
use instant_xml::{FromXml, ToXml};
use kumo_spf::SpfDisposition;
use serde::Serialize;
use std::fmt;
use std::net::IpAddr;

#[derive(Debug, Eq, FromXml, PartialEq, ToXml)]
#[xml(scalar, rename_all = "lowercase")]
pub enum SpfScope {
    Helo,
    Mfrom,
}

#[derive(Debug, Eq, FromXml, PartialEq, ToXml)]
#[xml(rename = "spf")]
pub struct SpfAuthResult {
    domain: String,
    scope: SpfScope,
    result: SpfDisposition,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "auth_results")]
pub struct AuthResults {
    dkim: Vec<DkimAuthResult>,
    spf: Vec<SpfAuthResult>,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "record")]
pub struct Results {
    row: Row,
    identifiers: Identifier,
    auth_results: AuthResults,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(scalar, rename_all = "lowercase")]
pub enum DkimResult {
    None,
    Pass,
    Fail,
    Policy,
    Neutral,
    TempError,
    PermError,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
pub struct DkimAuthResult {
    domain: String,
    selector: Option<String>,
    result: DkimResult,
    human_result: Option<String>,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, ToXml, Serialize)]
#[xml(scalar, rename_all = "lowercase")]
pub enum DmarcResult {
    Pass,
    Fail,
}

impl DmarcResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

impl fmt::Display for DmarcResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// A synthetic type to bundle the result with a reason
#[derive(Debug, Eq, PartialEq, ToXml, Serialize)]
#[xml(rename_all = "lowercase")]
pub struct DmarcResultWithContext {
    pub result: DmarcResult,
    pub context: String,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "policy_evaluated")]
pub struct PolicyEvaluated {
    disposition: Policy,
    dkim: DmarcResult,
    spf: DmarcResult,
    reason: Vec<PolicyOverrideReason>,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "row")]
pub struct Row {
    source_ip: IpAddr,
    count: u64,
    policy_evaluated: PolicyEvaluated,
}
