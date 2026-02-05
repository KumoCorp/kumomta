use instant_xml::ToXml;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ToXml)]
#[xml(scalar)]
pub(crate) enum Policy {
    None,
    Quarantine,
    Reject,
}

impl FromStr for Policy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "none" => Self::None,
            "quarantine" => Self::Quarantine,
            "reject" => Self::Reject,
            _ => return Err(format!("invalid policy {value:?}")),
        })
    }
}
