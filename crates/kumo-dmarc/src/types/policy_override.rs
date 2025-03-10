use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(scalar, rename_all = "lowercase")]
pub enum PolicyOverride {
    Forwarded,
    SampledOut,
    TrustedForwarder,
    MailingList,
    LocalPolicy,
    Other,
}

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "reason")]
pub struct PolicyOverrideReason {
    r#type: PolicyOverride,
    comment: Option<String>,
}
