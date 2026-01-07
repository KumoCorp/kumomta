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

impl ReportMetadata {
    pub fn new(
        org_name: String,
        email: String,
        extra_contact_info: Option<String>,
        report_id: String,
        date_range: DateRange,
        error: Vec<String>,
    ) -> Self {
        Self {
            org_name,
            email,
            extra_contact_info,
            report_id,
            date_range,
            error,
        }
    }
}
