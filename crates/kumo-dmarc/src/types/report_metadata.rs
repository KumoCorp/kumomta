use crate::types::date_range::DateRange;
use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
#[xml(rename = "report_metadata")]
pub struct ReportMetadata {
    org_name: String,
    email: String,
    extra_contact_info: Option<String>,
    report_id: String,
    date_range: DateRange,
    error: Vec<String>,
}
