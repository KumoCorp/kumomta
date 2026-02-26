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

impl PolicyPublished {
    pub fn new(
        domain: String,
        align_dkim: Option<Mode>,
        align_spf: Option<Mode>,
        policy: Policy,
        subdomain_policy: Policy,
        rate: u8,
        report_failure: ReportFailure,
    ) -> Self {
        PolicyPublished {
            domain,
            align_dkim,
            align_spf,
            policy,
            subdomain_policy,
            rate,
            report_failure,
        }
    }
}
