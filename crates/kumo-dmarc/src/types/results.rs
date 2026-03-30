use crate::types::identifier::Identifier;
use crate::types::policy::Policy;
use crate::types::policy_override::PolicyOverrideReason;
use bstr::{BString, ByteSlice};
use instant_xml::{FromXml, ToXml};
use kumo_spf::SpfDisposition;
use mailparsing::AuthenticationResult;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::IpAddr;

#[derive(Debug, Eq, FromXml, PartialEq, ToXml, Serialize, Deserialize, Clone, Copy)]
#[xml(scalar, rename_all = "lowercase")]
pub enum SpfScope {
    Helo,
    Mfrom,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub struct SpfAuthResult {
    domain: BString,
    scope: SpfScope,
    result: SpfDisposition,
}

impl ToXml for SpfAuthResult {
    fn serialize<W: fmt::Write + ?Sized>(
        &self,
        field: Option<instant_xml::Id<'_>>,
        serializer: &mut instant_xml::Serializer<W>,
    ) -> Result<(), instant_xml::Error> {
        #[derive(ToXml)]
        #[xml(rename = "spf")]
        struct SpfAuth {
            domain: String,
            scope: SpfScope,
            result: SpfDisposition,
        }

        SpfAuth {
            domain: self.domain.to_string(),
            scope: self.scope.clone(),
            result: self.result.clone(),
        }
        .serialize(field, serializer)
    }
}

impl From<AuthenticationResult> for SpfAuthResult {
    fn from(value: AuthenticationResult) -> Self {
        let d = value.props.get("header.d".as_bytes());

        Self {
            domain: d.cloned().unwrap_or_default(),
            scope: SpfScope::Mfrom,
            result: value.result.into(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone)]
#[xml(rename = "auth_results")]
pub struct AuthResults {
    pub(crate) dkim: Vec<DkimAuthResult>,
    pub(crate) spf: Vec<SpfAuthResult>,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "record")]
pub struct Results {
    pub(crate) row: Row,
    pub(crate) identifiers: Identifier,
    pub(crate) auth_results: AuthResults,
}

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone, Copy)]
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

impl From<String> for DkimResult {
    fn from(value: String) -> Self {
        match value.to_lowercase().as_str() {
            "none" => DkimResult::None,
            "pass" => DkimResult::Pass,
            "fail" => DkimResult::Fail,
            "policy" => DkimResult::Policy,
            "neutral" => DkimResult::Neutral,
            "temperror" => DkimResult::TempError,
            "permerror" => DkimResult::PermError,
            _ => DkimResult::None,
        }
    }
}

impl From<BString> for DkimResult {
    fn from(value: BString) -> Self {
        match value.to_lowercase().as_slice() {
            b"none" => DkimResult::None,
            b"pass" => DkimResult::Pass,
            b"fail" => DkimResult::Fail,
            b"policy" => DkimResult::Policy,
            b"neutral" => DkimResult::Neutral,
            b"temperror" => DkimResult::TempError,
            b"permerror" => DkimResult::PermError,
            _ => DkimResult::None,
        }
    }
}

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone)]
pub struct DkimAuthResult {
    domain: String,
    selector: Option<String>,
    result: DkimResult,
    human_result: Option<String>,
}

impl From<AuthenticationResult> for DkimAuthResult {
    fn from(value: AuthenticationResult) -> Self {
        let d = value.props.get("header.d".as_bytes());
        let s = value.props.get("header.s".as_bytes());
        Self {
            domain: d.cloned().unwrap_or_default().to_string(),
            selector: s.cloned().map(|x| x.to_string()),
            result: value.result.clone().into(),
            human_result: Some(value.result.to_string()),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, ToXml, Serialize, Deserialize)]
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

// A coverage of both success and various failure modes
#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Clone, Copy)]
#[xml(scalar)]
pub enum Disposition {
    Pass,
    None,
    Quarantine,
    Reject,
    TempError,
    PermError,
}

impl ToString for Disposition {
    fn to_string(&self) -> String {
        match self {
            Disposition::None => "None".to_string(),
            Disposition::Pass => "Pass".to_string(),
            Disposition::Quarantine => "Quarantine".to_string(),
            Disposition::Reject => "Reject".to_string(),
            Disposition::TempError => "TempError".to_string(),
            Disposition::PermError => "PermError".to_string(),
        }
    }
}

impl Into<Disposition> for Policy {
    fn into(self) -> Disposition {
        match self {
            Policy::None => Disposition::None,
            Policy::Quarantine => Disposition::Quarantine,
            Policy::Reject => Disposition::Reject,
        }
    }
}

// A synthetic type to bundle the result with a reason
#[derive(Debug, Eq, PartialEq, ToXml, Serialize)]
#[xml(rename_all = "lowercase")]
pub struct DispositionWithContext {
    pub result: Disposition,
    pub context: String,
}

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone)]
#[xml(rename = "policy_evaluated")]
pub struct PolicyEvaluated {
    pub(crate) disposition: Policy,
    pub(crate) dkim: DmarcResult,
    pub(crate) spf: DmarcResult,
    pub(crate) reason: Vec<PolicyOverrideReason>,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "row")]
pub struct Row {
    pub(crate) source_ip: IpAddr,
    pub(crate) count: u64,
    pub(crate) policy_evaluated: PolicyEvaluated,
}
