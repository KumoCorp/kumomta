use instant_xml::ToXml;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ToXml)]
#[xml(scalar)]
pub enum Mode {
    Relaxed,
    Strict,
}

impl From<Mode> for char {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Relaxed => 'r',
            Mode::Strict => 's',
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "r" => Self::Relaxed,
            "s" => Self::Strict,
            _ => return Err(format!("invalid mode {value:?}")),
        })
    }
}
