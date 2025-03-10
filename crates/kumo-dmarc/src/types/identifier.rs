use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
pub struct Identifier {
    envelope_to: Option<String>,
    envelope_from: String,
    header_from: String,
}
