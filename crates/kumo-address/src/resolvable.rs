use crate::socket::SocketAddress;
use hickory_proto::rr::Name;
use serde::{Deserialize, Serialize};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::unix::net::SocketAddr as UnixSocketAddr;
use std::str::FromStr;
use thiserror::Error;

/// An address that can be resolved to one or more concrete socket addresses.
///
/// Unlike [`SocketAddress`], the `Hostname` arm accepts a DNS name plus a
/// required port; resolution to a concrete IP requires a DNS lookup, which
/// is performed by `dns_resolver::resolve_socket_addr`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ResolvableSocketAddr {
    UnixDomain(Box<UnixSocketAddr>),
    V4(Box<SocketAddrV4>),
    V6(Box<SocketAddrV6>),
    /// A DNS name plus a required port. The host is stored verbatim
    /// (no IDNA normalization), but has been validated as a syntactically
    /// well-formed DNS name at construction time.
    Hostname {
        host: String,
        port: u16,
    },
}

/// Reason why a candidate string could not be parsed as a
/// [`ResolvableSocketAddr`]. Each field is the failure reason for one of the
/// three forms we attempt, in order: bracketed/literal IP socket, unix
/// domain socket path, and `host:port`.
#[derive(Error, Debug)]
#[error(
    "failed to parse {candidate:?} as a resolvable socket address: \
    not an IP socket ({socket}); \
    not a unix socket path ({unix}); \
    not host:port ({hostname})"
)]
pub struct ResolvableAddressParseError {
    pub candidate: String,
    pub socket: std::net::AddrParseError,
    pub unix: std::io::Error,
    pub hostname: HostnamePortParseError,
}

impl PartialEq for ResolvableAddressParseError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string().eq(&other.to_string())
    }
}

#[derive(Error, Debug)]
pub enum HostnamePortParseError {
    #[error("missing :port suffix")]
    MissingPort,
    #[error("invalid port {0:?}")]
    InvalidPort(String),
    #[error("invalid hostname {host:?}: {reason}")]
    InvalidHostname { host: String, reason: String },
}

impl std::fmt::Debug for ResolvableSocketAddr {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        <Self as std::fmt::Display>::fmt(self, fmt)
    }
}

impl std::fmt::Display for ResolvableSocketAddr {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::UnixDomain(unix) => match unix.as_pathname() {
                Some(path) => path.display().fmt(fmt),
                None => write!(fmt, "<unbound unix domain>"),
            },
            Self::V4(a) => a.fmt(fmt),
            Self::V6(a) => a.fmt(fmt),
            Self::Hostname { host, port } => write!(fmt, "{host}:{port}"),
        }
    }
}

impl ResolvableSocketAddr {
    /// Returns the unix domain socket representation of the address
    pub fn unix(&self) -> Option<UnixSocketAddr> {
        match self {
            Self::UnixDomain(unix) => Some((**unix).clone()),
            Self::V4(_) | Self::V6(_) | Self::Hostname { .. } => None,
        }
    }

    /// Returns the port number, if any. Unix domain sockets have no port.
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::UnixDomain(_) => None,
            Self::V4(a) => Some(a.port()),
            Self::V6(a) => Some(a.port()),
            Self::Hostname { port, .. } => Some(*port),
        }
    }
}

impl From<ResolvableSocketAddr> for String {
    fn from(a: ResolvableSocketAddr) -> String {
        format!("{a}")
    }
}

impl TryFrom<String> for ResolvableSocketAddr {
    type Error = ResolvableAddressParseError;
    fn try_from(s: String) -> Result<ResolvableSocketAddr, Self::Error> {
        ResolvableSocketAddr::from_str(&s)
    }
}

fn parse_hostname_port(s: &str) -> Result<(String, u16), HostnamePortParseError> {
    let (host, port) = s
        .rsplit_once(':')
        .ok_or(HostnamePortParseError::MissingPort)?;
    let port: u16 = port
        .parse()
        .map_err(|_| HostnamePortParseError::InvalidPort(port.to_string()))?;
    if host.is_empty() {
        return Err(HostnamePortParseError::InvalidHostname {
            host: host.to_string(),
            reason: "hostname is empty".to_string(),
        });
    }
    Name::from_str_relaxed(host).map_err(|e| HostnamePortParseError::InvalidHostname {
        host: host.to_string(),
        reason: e.to_string(),
    })?;
    Ok((host.to_string(), port))
}

impl FromStr for ResolvableSocketAddr {
    type Err = ResolvableAddressParseError;
    fn from_str(s: &str) -> Result<ResolvableSocketAddr, Self::Err> {
        match SocketAddress::from_str(s) {
            Ok(SocketAddress::UnixDomain(p)) => Ok(ResolvableSocketAddr::UnixDomain(p)),
            Ok(SocketAddress::V4(a)) => Ok(ResolvableSocketAddr::V4(Box::new(a))),
            Ok(SocketAddress::V6(a)) => Ok(ResolvableSocketAddr::V6(Box::new(a))),
            Err(socket_err) => match parse_hostname_port(s) {
                Ok((host, port)) => Ok(ResolvableSocketAddr::Hostname { host, port }),
                Err(hostname) => Err(ResolvableAddressParseError {
                    candidate: s.to_string(),
                    socket: socket_err.net_err,
                    unix: socket_err.unix_err,
                    hostname,
                }),
            },
        }
    }
}

impl PartialEq for ResolvableSocketAddr {
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
            (Self::Hostname { host: ah, port: ap }, Self::Hostname { host: bh, port: bp }) => {
                ah == bh && ap == bp
            }
            _ => false,
        }
    }
}

impl Eq for ResolvableSocketAddr {}

impl From<UnixSocketAddr> for ResolvableSocketAddr {
    fn from(unix: UnixSocketAddr) -> Self {
        Self::UnixDomain(unix.into())
    }
}

impl From<tokio::net::unix::SocketAddr> for ResolvableSocketAddr {
    fn from(unix: tokio::net::unix::SocketAddr) -> Self {
        let unix: UnixSocketAddr = unix.into();
        unix.into()
    }
}

impl From<SocketAddr> for ResolvableSocketAddr {
    fn from(a: SocketAddr) -> Self {
        match a {
            SocketAddr::V4(a) => Self::V4(Box::new(a)),
            SocketAddr::V6(a) => Self::V6(Box::new(a)),
        }
    }
}

impl From<SocketAddrV4> for ResolvableSocketAddr {
    fn from(a: SocketAddrV4) -> Self {
        Self::V4(Box::new(a))
    }
}

impl From<SocketAddrV6> for ResolvableSocketAddr {
    fn from(a: SocketAddrV6) -> Self {
        Self::V6(Box::new(a))
    }
}

impl From<SocketAddress> for ResolvableSocketAddr {
    fn from(a: SocketAddress) -> Self {
        match a {
            SocketAddress::UnixDomain(p) => Self::UnixDomain(p),
            SocketAddress::V4(a) => Self::V4(Box::new(a)),
            SocketAddress::V6(a) => Self::V6(Box::new(a)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn parse_literal_v4() {
        k9::assert_equal!(
            "10.0.0.1:25".parse::<ResolvableSocketAddr>().unwrap(),
            ResolvableSocketAddr::V4(Box::new(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 25)))
        );
        k9::assert_equal!(
            "[10.0.0.1]:25".parse::<ResolvableSocketAddr>().unwrap(),
            ResolvableSocketAddr::V4(Box::new(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 25)))
        );
    }

    #[test]
    fn parse_literal_v6() {
        k9::assert_equal!(
            "[::1]:100".parse::<ResolvableSocketAddr>().unwrap(),
            ResolvableSocketAddr::V6(Box::new(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 100, 0, 0)))
        );
    }

    #[test]
    fn parse_unix() {
        k9::assert_equal!(
            "/some/path".parse::<ResolvableSocketAddr>().unwrap(),
            ResolvableSocketAddr::UnixDomain(
                UnixSocketAddr::from_pathname("/some/path").unwrap().into()
            )
        );
    }

    #[test]
    fn parse_hostname() {
        k9::assert_equal!(
            "mx.example.com:25".parse::<ResolvableSocketAddr>().unwrap(),
            ResolvableSocketAddr::Hostname {
                host: "mx.example.com".to_string(),
                port: 25
            }
        );
    }

    #[test]
    fn reject_bogus_hostname() {
        let err = "I have spaces:25"
            .parse::<ResolvableSocketAddr>()
            .unwrap_err();
        k9::assert_equal!(
            format!("{err:#}"),
            "failed to parse \"I have spaces:25\" as a resolvable socket address: \
            not an IP socket (invalid socket address syntax); \
            not a unix socket path (unix domain path must be absolute); \
            not host:port (invalid hostname \"I have spaces\": unrecognized char:  )"
        );
    }

    #[test]
    fn reject_missing_port() {
        let err = "mx.example.com"
            .parse::<ResolvableSocketAddr>()
            .unwrap_err();
        k9::assert_equal!(
            format!("{err:#}"),
            "failed to parse \"mx.example.com\" as a resolvable socket address: \
            not an IP socket (invalid socket address syntax); \
            not a unix socket path (unix domain path must be absolute); \
            not host:port (missing :port suffix)"
        );
    }

    #[test]
    fn reject_bad_port() {
        let err = "mx.example.com:bogus"
            .parse::<ResolvableSocketAddr>()
            .unwrap_err();
        k9::assert_equal!(
            format!("{err:#}"),
            "failed to parse \"mx.example.com:bogus\" as a resolvable socket address: \
            not an IP socket (invalid socket address syntax); \
            not a unix socket path (unix domain path must be absolute); \
            not host:port (invalid port \"bogus\")"
        );
    }

    #[test]
    fn reject_unbracketed_v6_with_port() {
        // An unbracketed IPv6 literal followed by `:port` is ambiguous with
        // `host:port` and is not accepted by the standard socket parser.
        // Require the bracketed form (e.g. `[::1]:100`) instead.
        let err = "::1:100".parse::<ResolvableSocketAddr>().unwrap_err();
        k9::assert_equal!(
            format!("{err:#}"),
            "failed to parse \"::1:100\" as a resolvable socket address: \
            not an IP socket (invalid socket address syntax); \
            not a unix socket path (unix domain path must be absolute); \
            not host:port (invalid hostname \"::1\": Malformed label: ::1)"
        );

        let err = "fe80::1:100".parse::<ResolvableSocketAddr>().unwrap_err();
        k9::assert_equal!(
            format!("{err:#}"),
            "failed to parse \"fe80::1:100\" as a resolvable socket address: \
            not an IP socket (invalid socket address syntax); \
            not a unix socket path (unix domain path must be absolute); \
            not host:port (invalid hostname \"fe80::1\": Malformed label: fe80::1)"
        );
    }

    #[test]
    fn display_roundtrip() {
        for s in &[
            "10.0.0.1:25",
            "[::1]:100",
            "/some/path",
            "mx.example.com:25",
        ] {
            let a: ResolvableSocketAddr = s.parse().unwrap();
            let back: ResolvableSocketAddr = a.to_string().parse().unwrap();
            k9::assert_equal!(a, back);
        }
    }
}
