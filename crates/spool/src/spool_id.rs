use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

lazy_static::lazy_static! {
    static ref MAC: [u8;6] = get_mac_address();
}

fn get_mac_address() -> [u8; 6] {
    match mac_address::get_mac_address() {
        Ok(Some(addr)) => addr.bytes(),
        _ => {
            // Fall back to gethostid, which is not great, but
            // likely better than just random numbers
            let host_id = unsafe { libc::gethostid() }.to_le_bytes();
            let mac: [u8; 6] = [
                host_id[0], host_id[1], host_id[2], host_id[3], host_id[4], host_id[5],
            ];
            mac
        }
    }
}

/// Identifies a message within the spool of its host node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
#[derive(utoipa::ToSchema)]
#[schema(value_type=String, example="d7ef132b5d7711eea8c8000c29c33806")]
pub struct SpoolId(Uuid);

impl std::fmt::Display for SpoolId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.simple().fmt(fmt)
    }
}

impl From<Uuid> for SpoolId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<SpoolId> for String {
    fn from(id: SpoolId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for SpoolId {
    type Error = uuid::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let uuid = Uuid::parse_str(&s)?;
        Ok(Self(uuid))
    }
}

impl SpoolId {
    pub fn new() -> Self {
        // We're using v1, but we should be able to seamlessly upgrade to v7
        // once that feature stabilizes in the uuid crate
        Self(Uuid::now_v1(&*MAC))
    }

    pub fn compute_path(&self, in_dir: &Path) -> PathBuf {
        let (a, b, c, [d, e, f, g, h, i, j, k]) = self.0.as_fields();
        // Note that in a v1 UUID, a,b,c holds the timestamp components
        // from least-significant up to most significant.
        let [a1, a2, a3, a4] = a.to_be_bytes();
        let name = format!(
            "{a1:02x}/{a2:02x}/{a3:02x}/{a4:02x}/{b:04x}{c:04x}{d:02x}{e:02x}{f:02x}{g:02x}{h:02x}{i:02x}{j:02x}{k:02x}"
        );
        in_dir.join(name)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    pub fn from_slice(s: &[u8]) -> Option<Self> {
        let uuid = Uuid::from_slice(s).ok()?;
        Some(Self(uuid))
    }

    pub fn from_ascii_bytes(s: &[u8]) -> Option<Self> {
        let uuid = Uuid::try_parse_ascii(s).ok()?;
        Some(Self(uuid))
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let uuid = Uuid::parse_str(s).ok()?;
        Some(Self(uuid))
    }

    pub fn from_path(mut path: &Path) -> Option<Self> {
        let mut components = vec![];

        for _ in 0..5 {
            components.push(path.file_name()?.to_str()?);
            path = path.parent()?;
        }

        components.reverse();
        Some(Self(Uuid::parse_str(&components.join("")).ok()?))
    }

    /// Returns time elapsed since the id was created,
    /// given the current timestamp
    pub fn age(&self, now: DateTime<Utc>) -> Duration {
        let created = self.created();
        now - created
    }

    pub fn created(&self) -> DateTime<Utc> {
        let (seconds, nanos) = self.0.get_timestamp().unwrap().to_unix();
        Utc.timestamp_opt(seconds.try_into().unwrap(), nanos)
            .unwrap()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn roundtrip_path() {
        let id = SpoolId::new();
        eprintln!("{id}");
        let path = id.compute_path(Path::new("."));
        let id2 = SpoolId::from_path(&path).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn roundtrip_bytes() {
        let id = SpoolId::new();
        eprintln!("{id}");
        let bytes = id.as_bytes();
        let id2 = SpoolId::from_slice(bytes.as_slice()).unwrap();
        assert_eq!(id, id2);
    }
}
