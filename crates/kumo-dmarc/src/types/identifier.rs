use instant_xml::ToXml;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, ToXml, Serialize, Deserialize, Clone)]
pub struct Identifier {
    pub(crate) envelope_to: Vec<String>,
    pub(crate) envelope_from: Vec<String>,
    pub(crate) header_from: String,
}
