use instant_xml::ToXml;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone, Copy)]
#[xml(scalar, rename_all = "lowercase")]
pub enum PolicyOverride {
    Forwarded,
    SampledOut,
    TrustedForwarder,
    MailingList,
    LocalPolicy,
    Other,
}

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone)]
#[xml(rename = "reason")]
pub struct PolicyOverrideReason {
    r#type: PolicyOverride,
    comment: Option<String>,
}
