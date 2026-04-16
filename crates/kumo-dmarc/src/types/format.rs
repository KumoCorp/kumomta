use std::str::FromStr;

#[derive(Debug)]
pub enum Format {
    Afrf,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "afrf" => Self::Afrf,
            _ => return Err(format!("invalid format {value:?}")),
        })
    }
}
