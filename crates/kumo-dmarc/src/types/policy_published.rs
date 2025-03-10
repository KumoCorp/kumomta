use crate::types::mode::Mode;
use crate::types::policy::Policy;
use crate::types::report_failure::ReportFailure;
use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "policy_published")]
pub struct PolicyPublished {
    domain: String,
    #[xml(rename = "adkim")]
    align_dkim: Option<Mode>,
    #[xml(rename = "aspf")]
    align_spf: Option<Mode>,
    #[xml(rename = "p")]
    policy: Policy,
    #[xml(rename = "sp")]
    subdomain_policy: Policy,
    #[xml(rename = "pct")]
    rate: u8,
    #[xml(rename = "fo")]
    report_failure: ReportFailure,
}
