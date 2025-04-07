use crate::host::{AddressParseError, HostAddress};
use crate::socket::SocketAddress;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::unix::net::SocketAddr as UnixSocketAddr;
use std::str::FromStr;

/// Specify either a unix or IP address. The IP address can optionally
/// include a port number
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum HostOrSocketAddress {
    UnixDomain(Box<UnixSocketAddr>),
    V4Socket(Box<SocketAddrV4>),
    V6Socket(Box<SocketAddrV6>),
    V4Host(std::net::Ipv4Addr),
    V6Host(std::net::Ipv6Addr),
}

impl std::fmt::Debug for HostOrSocketAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        <Self as std::fmt::Display>::fmt(self, fmt)
    }
}

impl std::fmt::Display for HostOrSocketAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::UnixDomain(unix) => match unix.as_pathname() {
                Some(path) => path.display().fmt(fmt),
                None => write!(fmt, "<unbound unix domain>"),
            },
            Self::V4Socket(a) => a.fmt(fmt),
            Self::V6Socket(a) => a.fmt(fmt),
            Self::V4Host(a) => a.fmt(fmt),
            Self::V6Host(a) => a.fmt(fmt),
        }
    }
}

impl From<SocketAddress> for HostOrSocketAddress {
    fn from(a: SocketAddress) -> HostOrSocketAddress {
        match a {
            SocketAddress::UnixDomain(unix) => HostOrSocketAddress::UnixDomain(unix),
            SocketAddress::V4(v4) => HostOrSocketAddress::V4Socket(v4.into()),
            SocketAddress::V6(v6) => HostOrSocketAddress::V6Socket(v6.into()),
        }
    }
}

impl From<HostAddress> for HostOrSocketAddress {
    fn from(a: HostAddress) -> HostOrSocketAddress {
        match a {
            HostAddress::UnixDomain(unix) => HostOrSocketAddress::UnixDomain(unix),
            HostAddress::V4(v4) => HostOrSocketAddress::V4Host(v4),
            HostAddress::V6(v6) => HostOrSocketAddress::V6Host(v6),
        }
    }
}

impl From<HostOrSocketAddress> for String {
    fn from(a: HostOrSocketAddress) -> String {
        format!("{a}")
    }
}

impl TryFrom<String> for HostOrSocketAddress {
    type Error = AddressParseError;
    fn try_from(s: String) -> Result<HostOrSocketAddress, Self::Error> {
        HostOrSocketAddress::from_str(&s)
    }
}

impl HostOrSocketAddress {
    /// Returns the "host" portion of the address
    pub fn host(&self) -> HostAddress {
        match self {
            Self::UnixDomain(p) => HostAddress::UnixDomain(p.clone()),
            Self::V4Host(a) => HostAddress::V4(*a),
            Self::V4Socket(a) => HostAddress::V4(*a.ip()),
            Self::V6Host(a) => HostAddress::V6(*a),
            Self::V6Socket(a) => HostAddress::V6(*a.ip()),
        }
    }

    /// Returns the unix domain socket representation of the address
    pub fn unix(&self) -> Option<UnixSocketAddr> {
        match self {
            Self::UnixDomain(unix) => Some((**unix).clone()),
            Self::V4Host(_) | Self::V6Host(_) | Self::V4Socket(_) | Self::V6Socket(_) => None,
        }
    }

    /// Returns the ip representation of the address
    pub fn ip(&self) -> Option<IpAddr> {
        match self {
            Self::UnixDomain(_) => None,
            Self::V4Host(a) => Some((*a).into()),
            Self::V4Socket(a) => Some((*a.ip()).into()),
            Self::V6Host(a) => Some((*a).into()),
            Self::V6Socket(a) => Some((*a.ip()).into()),
        }
    }

    /// Returns the ip SocketAddr (including port) representation of the address
    pub fn ip_and_port(&self) -> Option<SocketAddr> {
        match self {
            Self::UnixDomain(_) => None,
            Self::V4Host(_) | Self::V6Host(_) => None,
            Self::V4Socket(a) => Some((*a.clone()).into()),
            Self::V6Socket(a) => Some((*a.clone()).into()),
        }
    }

    /// Returns the port number, if specified
    pub fn port(&self) -> Option<u16> {
        self.ip_and_port().map(|sa| sa.port())
    }

    /// Assign a new port number
    pub fn set_port(&mut self, port: u16) {
        match self {
            HostOrSocketAddress::UnixDomain(_) => {}
            HostOrSocketAddress::V6Socket(s) => {
                s.set_port(port);
            }
            HostOrSocketAddress::V4Socket(s) => {
                s.set_port(port);
            }
            HostOrSocketAddress::V4Host(v4) => {
                *self = HostOrSocketAddress::V4Socket(Box::new(SocketAddrV4::new(*v4, port)));
            }
            HostOrSocketAddress::V6Host(v6) => {
                *self = HostOrSocketAddress::V6Socket(Box::new(SocketAddrV6::new(*v6, port, 0, 0)));
            }
        }
    }

    /// Assign a port number if no port is already set
    pub fn set_port_if_not_set(&mut self, port: u16) {
        match self {
            HostOrSocketAddress::UnixDomain(_) => {}
            HostOrSocketAddress::V6Socket(_) | HostOrSocketAddress::V4Socket(_) => {
                // Already has a port: don't override
            }
            HostOrSocketAddress::V4Host(v4) => {
                *self = HostOrSocketAddress::V4Socket(Box::new(SocketAddrV4::new(*v4, port)));
            }
            HostOrSocketAddress::V6Host(v6) => {
                *self = HostOrSocketAddress::V6Socket(Box::new(SocketAddrV6::new(*v6, port, 0, 0)));
            }
        }
    }
}

impl FromStr for HostOrSocketAddress {
    type Err = AddressParseError;
    fn from_str(s: &str) -> Result<HostOrSocketAddress, Self::Err> {
        match SocketAddress::from_str(s) {
            Ok(sa) => Ok(sa.into()),
            Err(sa_err) => match HostAddress::from_str(s) {
                Ok(ha) => Ok(ha.into()),
                Err(_ha_err) => Err(sa_err),
            },
        }
    }
}

impl PartialEq for HostOrSocketAddress {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UnixDomain(a), Self::UnixDomain(b)) => {
                match (a.as_pathname(), b.as_pathname()) {
                    (Some(a), Some(b)) => a.eq(b),
                    (None, None) => true,
                    _ => false,
                }
            }
            (Self::V4Socket(a), Self::V4Socket(b)) => a.eq(b),
            (Self::V6Socket(a), Self::V6Socket(b)) => a.eq(b),
            (Self::V4Host(a), Self::V4Host(b)) => a.eq(b),
            (Self::V6Host(a), Self::V6Host(b)) => a.eq(b),
            _ => false,
        }
    }
}

impl Eq for HostOrSocketAddress {}

impl From<UnixSocketAddr> for HostOrSocketAddress {
    fn from(unix: UnixSocketAddr) -> HostOrSocketAddress {
        HostOrSocketAddress::UnixDomain(unix.into())
    }
}

impl From<IpAddr> for HostOrSocketAddress {
    fn from(ip: IpAddr) -> HostOrSocketAddress {
        match ip {
            IpAddr::V4(a) => HostOrSocketAddress::V4Host(a),
            IpAddr::V6(a) => HostOrSocketAddress::V6Host(a),
        }
    }
}

impl From<Ipv4Addr> for HostOrSocketAddress {
    fn from(ip: Ipv4Addr) -> HostOrSocketAddress {
        HostOrSocketAddress::V4Host(ip)
    }
}

impl From<Ipv6Addr> for HostOrSocketAddress {
    fn from(ip: Ipv6Addr) -> HostOrSocketAddress {
        HostOrSocketAddress::V6Host(ip)
    }
}

impl From<SocketAddr> for HostOrSocketAddress {
    fn from(ip: SocketAddr) -> HostOrSocketAddress {
        match ip {
            SocketAddr::V4(a) => HostOrSocketAddress::V4Socket(a.into()),
            SocketAddr::V6(a) => HostOrSocketAddress::V6Socket(a.into()),
        }
    }
}

impl From<tokio::net::unix::SocketAddr> for HostOrSocketAddress {
    fn from(unix: tokio::net::unix::SocketAddr) -> HostOrSocketAddress {
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
            "10.0.0.1:25".parse::<HostOrSocketAddress>(),
            Ok(HostOrSocketAddress::V4Socket(
                SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 25).into()
            ))
        );
        assert_eq!(
            "[10.0.0.1]:25".parse::<HostOrSocketAddress>(),
            Ok(HostOrSocketAddress::V4Socket(
                SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 25).into()
            ))
        );
        assert_eq!(
            "[::1]:100".parse::<HostOrSocketAddress>(),
            Ok(HostOrSocketAddress::V6Socket(
                SocketAddrV6::new(Ipv6Addr::LOCALHOST, 100, 0, 0).into()
            ))
        );
        assert_eq!(
            "/some/path".parse::<HostOrSocketAddress>(),
            Ok(HostOrSocketAddress::UnixDomain(
                UnixSocketAddr::from_pathname("/some/path").unwrap().into()
            ))
        );
        assert_eq!(
            "[10.0.0.1]".parse::<HostOrSocketAddress>(),
            Ok(HostOrSocketAddress::V4Host(Ipv4Addr::new(10, 0, 0, 1),))
        );
        assert_eq!(
            "[::1]".parse::<HostOrSocketAddress>(),
            Ok(HostOrSocketAddress::V6Host(Ipv6Addr::LOCALHOST))
        );
        assert_eq!(
            format!(
                "{:#}",
                "hello there".parse::<HostOrSocketAddress>().unwrap_err()
            ),
            "Failed to parse 'hello there' as an address. \
            Got 'invalid socket address syntax' when considering it as \
            an IP address and 'unix domain path must be absolute' \
            when considering it as a unix domain socket path."
        );
        assert_eq!(
            format!(
                "{:#}",
                "[10.0.0.1]:bogus"
                    .parse::<HostOrSocketAddress>()
                    .unwrap_err()
            ),
            "Failed to parse '[10.0.0.1]:bogus' as an address. \
            Got 'invalid socket address syntax' when considering it as \
            an IP address and 'unix domain path must be absolute' \
            when considering it as a unix domain socket path."
        );
    }
}
