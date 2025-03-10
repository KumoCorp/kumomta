use std::str::FromStr;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FeedbackAddress {
    pub uri: String,
    pub size: Option<u64>,
}

impl FromStr for FeedbackAddress {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty feedback address {s:?}".to_owned());
        }

        let Some((uri, size)) = s.trim().rsplit_once('!') else {
            return Ok(Self {
                uri: s.to_owned(),
                size: None,
            });
        };

        let size = size.trim();
        if size.is_empty() {
            return Err(format!("empty size in {s:?}"));
        }

        let mut power = 0;
        match size.chars().next_back() {
            Some('k') => power = 10,
            Some('m') => power = 20,
            Some('g') => power = 30,
            Some('t') => power = 40,
            _ => {}
        }

        let size = match power {
            0 => size,
            _ => &size[..size.len() - 1],
        };

        let size = u64::from_str(size).map_err(|_| format!("invalid size in {s:?}"))? << power;
        Ok(Self {
            uri: uri.to_owned(),
            size: Some(size),
        })
    }
}
