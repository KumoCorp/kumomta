pub use cidr::IpCidr;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// A little helper struct to reduce the boilerplate when
/// checking against a list of cidrs
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct CidrSet(Vec<IpCidr>);

impl CidrSet {
    pub fn new(set: Vec<IpCidr>) -> Self {
        Self(set)
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        for entry in &self.0 {
            if entry.contains(&ip) {
                return true;
            }
        }
        false
    }
}
