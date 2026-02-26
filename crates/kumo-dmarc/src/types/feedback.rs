use crate::types::policy_published::PolicyPublished;
use crate::types::report_metadata::ReportMetadata;
use crate::types::results::Results;
use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
pub struct Feedback {
    pub(crate) version: String,
    pub(crate) metadata: ReportMetadata,
    pub(crate) policy: PolicyPublished,
    pub(crate) record: Vec<Results>,
}

impl Feedback {
    pub fn new(
        version: String,
        metadata: ReportMetadata,
        policy: PolicyPublished,
        record: Vec<Results>,
    ) -> Self {
        Self {
            version,
            metadata,
            policy,
            record,
        }
    }
}
