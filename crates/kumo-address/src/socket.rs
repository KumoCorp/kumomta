use crate::host::{AddressParseError, HostAddress};
use serde::{Deserialize, Serialize};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::unix::net::SocketAddr as UnixSocketAddr;
use std::path::Path;
use std::str::FromStr;

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum SocketAddress {
    UnixDomain(Box<UnixSocketAddr>),
    V4(std::net::SocketAddrV4),
    V6(std::net::SocketAddrV6),
}

impl std::fmt::Debug for SocketAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        <Self as std::fmt::Display>::fmt(self, fmt)
    }
}

impl std::fmt::Display for SocketAddress {
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

impl From<SocketAddress> for String {
    fn from(a: SocketAddress) -> String {
        format!("{a}")
    }
}

impl TryFrom<String> for SocketAddress {
    type Error = AddressParseError;
    fn try_from(s: String) -> Result<SocketAddress, Self::Error> {
        SocketAddress::from_str(&s)
    }
}

impl SocketAddress {
    /// Returns the "host" portion of the address
    pub fn host(&self) -> HostAddress {
        match self {
            Self::UnixDomain(p) => HostAddress::UnixDomain(p.clone()),
            Self::V4(a) => HostAddress::V4(a.ip().clone()),
            Self::V6(a) => HostAddress::V6(a.ip().clone()),
        }
    }

    /// Returns the unix domain socket representation of the address
    pub fn unix(&self) -> Option<UnixSocketAddr> {
        match self {
            Self::V4(_) | Self::V6(_) => None,
            Self::UnixDomain(unix) => Some((**unix).clone()),
        }
    }

    /// Returns the ip representation of the address
    pub fn ip(&self) -> Option<SocketAddr> {
        match self {
            Self::V4(a) => Some(a.clone().into()),
            Self::V6(a) => Some(a.clone().into()),
            Self::UnixDomain(_) => None,
        }
    }
}

impl FromStr for SocketAddress {
    type Err = AddressParseError;
    fn from_str(s: &str) -> Result<SocketAddress, Self::Err> {
        // At the time of writing, Rust's IPv6 SockAddr parsing
        // interally only accepts `[address]:port` while its IPv4
        // SockAddr parsing only accepts `address:port`.
        // In the email world, `[]` is used to indicate a literal
        // IP address so we desire the ability to uniformly use
        // the `[]` syntax in both cases, so we check for that
        // first and parse the internal address out.

        if s.starts_with('[') {
            if let Some(host_end) = s.find(']') {
                let (host, remainder) = s.split_at(host_end);
                let host = &host[1..];

                if let Some(port) = remainder.strip_prefix("]:") {
                    if let Ok(port) = port.parse::<u16>() {
                        match HostAddress::from_str(host) {
                            Ok(HostAddress::V4(a)) => {
                                return Ok(SocketAddress::V4(SocketAddrV4::new(a, port)))
                            }
                            Ok(HostAddress::V6(a)) => {
                                return Ok(SocketAddress::V6(SocketAddrV6::new(a, port, 0, 0)))
                            }

                            _ => {}
                        }
                    }
                }
            }
        }

        match SocketAddr::from_str(s) {
            Ok(a) => Ok(a.into()),
            Err(net_err) => {
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
                        Ok(unix) => Ok(SocketAddress::UnixDomain(unix.into())),
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

impl PartialEq for SocketAddress {
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

impl Eq for SocketAddress {}

impl From<UnixSocketAddr> for SocketAddress {
    fn from(unix: UnixSocketAddr) -> SocketAddress {
        SocketAddress::UnixDomain(unix.into())
    }
}

impl From<SocketAddr> for SocketAddress {
    fn from(ip: SocketAddr) -> SocketAddress {
        match ip {
            SocketAddr::V4(a) => SocketAddress::V4(a),
            SocketAddr::V6(a) => SocketAddress::V6(a),
        }
    }
}

impl From<tokio::net::unix::SocketAddr> for SocketAddress {
    fn from(unix: tokio::net::unix::SocketAddr) -> SocketAddress {
        let unix: UnixSocketAddr = unix.into();
        unix.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn parse() {
        assert_eq!(
            "10.0.0.1:25".parse::<SocketAddress>(),
            Ok(SocketAddress::V4(SocketAddrV4::new(
                Ipv4Addr::new(10, 0, 0, 1),
                25
            )))
        );
        assert_eq!(
            "[10.0.0.1]:25".parse::<SocketAddress>(),
            Ok(SocketAddress::V4(SocketAddrV4::new(
                Ipv4Addr::new(10, 0, 0, 1),
                25
            )))
        );
        assert_eq!(
            "[::1]:100".parse::<SocketAddress>(),
            Ok(SocketAddress::V6(SocketAddrV6::new(
                Ipv6Addr::LOCALHOST,
                100,
                0,
                0
            )))
        );
        assert_eq!(
            "/some/path".parse::<SocketAddress>(),
            Ok(SocketAddress::UnixDomain(
                UnixSocketAddr::from_pathname("/some/path").unwrap().into()
            ))
        );
        assert_eq!(
            format!("{:#}", "hello there".parse::<SocketAddress>().unwrap_err()),
            "Failed to parse 'hello there' as an address. \
            Got 'invalid socket address syntax' when considering it as \
            an IP address and 'unix domain path must be absolute' \
            when considering it as a unix domain socket path."
        );
        assert_eq!(
            format!("{:#}", "[10.0.0.1]".parse::<SocketAddress>().unwrap_err()),
            "Failed to parse '[10.0.0.1]' as an address. \
            Got 'invalid socket address syntax' when considering it as \
            an IP address and 'unix domain path must be absolute' \
            when considering it as a unix domain socket path."
        );
    }
}
