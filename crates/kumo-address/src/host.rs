use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::net::SocketAddr as UnixSocketAddr;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug)]
#[error(
    "Failed to parse '{candidate}' as an address. \
    Got '{net_err}' when considering it as an IP address and \
    '{unix_err}' when considering it as a unix domain socket path."
)]
pub struct AddressParseError {
    pub(crate) candidate: String,
    pub(crate) net_err: std::net::AddrParseError,
    pub(crate) unix_err: std::io::Error,
}

impl PartialEq for AddressParseError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string().eq(&other.to_string())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub enum HostAddress {
    UnixDomain(Box<UnixSocketAddr>),
    V4(std::net::Ipv4Addr),
    V6(std::net::Ipv6Addr),
}

impl HostAddress {
    /// Returns the unix domain socket representation of the address
    pub fn unix(&self) -> Option<UnixSocketAddr> {
        match self {
            Self::V4(_) | Self::V6(_) => None,
            Self::UnixDomain(unix) => Some((**unix).clone()),
        }
    }

    /// Returns the ip representation of the address
    pub fn ip(&self) -> Option<IpAddr> {
        match self {
            Self::V4(a) => Some((*a).into()),
            Self::V6(a) => Some((*a).into()),
            Self::UnixDomain(_) => None,
        }
    }
}

impl PartialEq for HostAddress {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UnixDomain(a), Self::UnixDomain(b)) => {
                match (a.as_pathname(), b.as_pathname()) {
                    (Some(a), Some(b)) => a.eq(b),
                    (None, None) => true,
                    _ => false,
                }
            }
            (Self::V4(a), Self::V4(b)) => a.eq(b),
            (Self::V6(a), Self::V6(b)) => a.eq(b),
            _ => false,
        }
    }
}

impl Eq for HostAddress {}

impl From<HostAddress> for String {
    fn from(a: HostAddress) -> String {
        format!("{a}")
    }
}

impl std::fmt::Debug for HostAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        <Self as std::fmt::Display>::fmt(self, fmt)
    }
}

impl std::fmt::Display for HostAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::UnixDomain(unix) => match unix.as_pathname() {
                Some(path) => path.display().fmt(fmt),
                None => write!(fmt, "<unbound unix domain>"),
            },
            Self::V4(a) => a.fmt(fmt),
            Self::V6(a) => a.fmt(fmt),
        }
    }
}

impl FromStr for HostAddress {
    type Err = AddressParseError;
    fn from_str(s: &str) -> Result<HostAddress, Self::Err> {
        match IpAddr::from_str(s) {
            Ok(a) => Ok(a.into()),
            Err(net_err) => {
                if s.starts_with('[') && s.ends_with(']') {
                    let alternative = &s[1..s.len() - 1];
                    if let Ok(a) = IpAddr::from_str(alternative) {
                        return Ok(a.into());
                    }
                }

                let path: &Path = s.as_ref();
                if path.is_relative() {
                    Err(AddressParseError {
                        candidate: s.to_string(),
                        net_err,
                        unix_err: std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "unix domain path must be absolute",
                        ),
                    })
                } else {
                    match UnixSocketAddr::from_pathname(path) {
                        Ok(unix) => Ok(HostAddress::UnixDomain(unix.into())),
                        Err(unix_err) => Err(AddressParseError {
                            candidate: s.to_string(),
                            net_err,
                            unix_err,
                        }),
                    }
                }
            }
        }
    }
}

impl TryFrom<String> for HostAddress {
    type Error = AddressParseError;
    fn try_from(s: String) -> Result<HostAddress, Self::Error> {
        HostAddress::from_str(&s)
    }
}

impl From<UnixSocketAddr> for HostAddress {
    fn from(unix: UnixSocketAddr) -> HostAddress {
        HostAddress::UnixDomain(unix.into())
    }
}

impl From<Ipv4Addr> for HostAddress {
    fn from(ip: Ipv4Addr) -> HostAddress {
        HostAddress::V4(ip)
    }
}

impl From<Ipv6Addr> for HostAddress {
    fn from(ip: Ipv6Addr) -> HostAddress {
        HostAddress::V6(ip)
    }
}

impl From<IpAddr> for HostAddress {
    fn from(ip: IpAddr) -> HostAddress {
        match ip {
            IpAddr::V4(a) => HostAddress::V4(a),
            IpAddr::V6(a) => HostAddress::V6(a),
        }
    }
}

impl From<SocketAddr> for HostAddress {
    fn from(a: SocketAddr) -> HostAddress {
        a.ip().into()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse() {
        assert_eq!(
            "10.0.0.1".parse::<HostAddress>(),
            Ok(HostAddress::V4(Ipv4Addr::new(10, 0, 0, 1)))
        );
        assert_eq!(
            "[10.0.0.1]".parse::<HostAddress>(),
            Ok(HostAddress::V4(Ipv4Addr::new(10, 0, 0, 1)))
        );
        assert_eq!(
            "::1".parse::<HostAddress>(),
            Ok(HostAddress::V6(Ipv6Addr::LOCALHOST))
        );
        assert_eq!(
            "[::1]".parse::<HostAddress>(),
            Ok(HostAddress::V6(Ipv6Addr::LOCALHOST))
        );
        assert_eq!(
            "/some/path".parse::<HostAddress>(),
            Ok(HostAddress::UnixDomain(
                UnixSocketAddr::from_pathname("/some/path").unwrap().into()
            ))
        );
        assert_eq!(
            format!("{:#}", "[/some/path]".parse::<HostAddress>().unwrap_err()),
            "Failed to parse '[/some/path]' as an address. \
            Got 'invalid IP address syntax' when considering it as \
            an IP address and 'unix domain path must be absolute' \
            when considering it as a unix domain socket path."
        );
        assert_eq!(
            format!("{:#}", "hello there".parse::<HostAddress>().unwrap_err()),
            "Failed to parse 'hello there' as an address. \
            Got 'invalid IP address syntax' when considering it as \
            an IP address and 'unix domain path must be absolute' \
            when considering it as a unix domain socket path."
        );
    }
}
