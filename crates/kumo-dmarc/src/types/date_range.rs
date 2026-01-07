use chrono::{DateTime, Utc};
use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq)]
pub struct DateRange {
    begin: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl DateRange {
    pub fn new(begin: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self { begin, end }
    }
}

impl ToXml for DateRange {
    fn serialize<W: std::fmt::Write + ?Sized>(
        &self,
        field: Option<instant_xml::Id<'_>>,
        serializer: &mut instant_xml::Serializer<W>,
    ) -> Result<(), instant_xml::Error> {
        #[derive(ToXml)]
        #[xml(rename = "date_range")]
        struct Timestamps {
            begin: i64,
            end: i64,
        }

        Timestamps {
            begin: self.begin.timestamp(),
            end: self.end.timestamp(),
        }
        .serialize(field, serializer)
    }
}
