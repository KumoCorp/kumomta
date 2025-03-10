use instant_xml::ToXml;
use std::fmt::Write;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReportFailure {
    all_pass: bool,
    any_pass: bool,
    dkim: bool,
    spf: bool,
}

impl ToXml for ReportFailure {
    fn serialize<W: std::fmt::Write + ?Sized>(
        &self,
        _: Option<instant_xml::Id<'_>>,
        serializer: &mut instant_xml::Serializer<W>,
    ) -> Result<(), instant_xml::Error> {
        serializer.write_str(self)
    }
}

impl std::fmt::Display for ReportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let values = [
            (self.all_pass, '0'),
            (self.any_pass, '1'),
            (self.dkim, 'd'),
            (self.spf, 's'),
        ];

        let mut first = true;
        for (value, ch) in values.into_iter() {
            if !value {
                continue;
            }

            if !first {
                f.write_char(':')?;
            } else {
                first = false;
            }

            f.write_char(ch)?;
        }

        Ok(())
    }
}

impl FromStr for ReportFailure {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut new = Self::default();
        for part in value.split(':') {
            match part.trim() {
                "0" => new.all_pass = true,
                "1" => new.any_pass = true,
                "d" => new.dkim = true,
                "s" => new.spf = true,
                _ => return Err(format!("invalid report failure {value:?}")),
            }
        }

        Ok(new)
    }
}

impl Default for ReportFailure {
    fn default() -> Self {
        Self {
            all_pass: true,
            any_pass: false,
            dkim: false,
            spf: false,
        }
    }
}
