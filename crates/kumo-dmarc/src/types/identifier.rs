use instant_xml::ToXml;

#[derive(Debug, Eq, PartialEq, ToXml)]
pub struct Identifier {
    pub(crate) envelope_to: Vec<String>,
    pub(crate) envelope_from: Vec<String>,
    pub(crate) header_from: String,
}
