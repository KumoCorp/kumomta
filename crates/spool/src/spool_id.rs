use chrono::{DateTime, Duration, TimeZone, Utc};
use std::path::{Path, PathBuf};
use uuid::Uuid;

lazy_static::lazy_static! {
    static ref MAC: [u8;6] = get_mac_address();
}

fn get_mac_address() -> [u8; 6] {
    match mac_address::get_mac_address() {
        Ok(Some(addr)) => addr.bytes(),
        _ => {
            let mut mac = [0u8; 6];
            getrandom::getrandom(&mut mac).ok();
            mac
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpoolId(Uuid);

impl std::fmt::Display for SpoolId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.simple().fmt(fmt)
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
        let name = format!(
            "{a:08x}/{b:04x}/{c:04x}/{d:02x}{e:02x}/{f:02x}{g:02x}{h:02x}{i:02x}{j:02x}{k:02x}"
        );
        in_dir.join(name)
    }

    pub fn from_path(mut path: &Path) -> Option<Self> {
        let mut components = vec![];

        for _ in 0..5 {
            components.push(path.file_name()?.to_str()?);
            path = path.parent()?;
        }

        components.reverse();
        Some(Self(Uuid::try_parse(&components.join("-")).ok()?))
    }

    /// Returns time elapsed since the id was created,
    /// given the current timestamp
    pub fn age(&self, now: DateTime<Utc>) -> Duration {
        let (seconds, nanos) = self.0.get_timestamp().unwrap().to_unix();
        let created = Utc
            .timestamp_opt(seconds.try_into().unwrap(), nanos)
            .unwrap();
        now - created
    }
}
