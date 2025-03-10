use crate::types::identifier::Identifier;
use crate::types::policy::Policy;
use crate::types::policy_override::PolicyOverrideReason;
use instant_xml::{FromXml, ToXml};
use kumo_spf::SpfDisposition;
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

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(scalar, rename_all = "lowercase")]
pub enum DmarcResult {
    Pass,
    Fail,
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
