use crate::types::policy_published::PolicyPublished;
use crate::types::report_metadata::ReportMetadata;
use crate::types::results::Results;
use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
pub struct Feedback {
    version: String,
    metadata: ReportMetadata,
    policy: PolicyPublished,
    record: Vec<Results>,
}
