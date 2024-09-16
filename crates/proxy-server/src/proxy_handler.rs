use anyhow::Context;
use socksv5::v5::{
    SocksV5AuthMethod, SocksV5Command, SocksV5Host, SocksV5Request, SocksV5RequestStatus,
};
use std::net::{IpAddr, SocketAddr};
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::timeout;

/// Given a newly accepted client, perform a SOCKS5 handshake and then
/// run the proxy logic, passing data from the client through to the
/// host to which we have connected on the behalf of the client.
pub async fn handle_proxy_client(
    mut stream: TcpStream,
    peer_address: SocketAddr,
    timeout_duration: std::time::Duration,
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    no_splice: bool,
) -> anyhow::Result<()> {
    let mut state = ClientState::None;

    let handshake = timeout(timeout_duration, async {
        socksv5::v5::read_handshake(&mut stream).await
    })
    .await
    .with_context(|| format!("timeout reading client handshake from {peer_address:?}"))?
    .with_context(|| format!("failed to read client handshake from {peer_address:?}"))?;

    if handshake
        .methods
        .into_iter()
        .find(|m| *m == socksv5::v5::SocksV5AuthMethod::Noauth)
        .is_none()
    {
        return Err(anyhow::anyhow!(
            "client {peer_address:?} requested authentication, but this proxy only supports NOAUTH"
        ));
    }

    timeout(timeout_duration, async {
        socksv5::v5::write_auth_method(&mut stream, SocksV5AuthMethod::Noauth).await
    })
    .await
    .with_context(|| format!("timeout sending Noauth response to {peer_address:?}"))?
    .with_context(|| format!("failed to send Noauth response to {peer_address:?}"))?;

    loop {
        let request = timeout(timeout_duration, async {
            socksv5::v5::read_request(&mut stream).await
        })
        .await
        .with_context(|| format!("timeout reading request from {peer_address:?} {state:?}"))?
        .with_context(|| format!("failed reading request from {peer_address:?} {state:?}"))?;

        log::trace!("peer={peer_address:?} request: {request:?}");

        let status = match timeout(timeout_duration, async {
            handle_request(&request, &mut state).await
        })
        .await
        {
            Err(_) => RequestStatus::timeout(),
            Ok(Ok(s)) => s,
            Ok(Err(err)) => {
                log::error!("peer={peer_address:?}: {state:?} {request:?} -> {err:#}");
                RequestStatus::error(err)
            }
        };

        // socks5 crate doesn't believe in allowing cloning, so we premptively debug
        // dump the status in case of failure
        let status_debug = format!("{status:?}");

        let is_success = status.status == SocksV5RequestStatus::Success;
        log::trace!("peer={peer_address:?}: {state:?} status -> {status:?}");

        timeout(timeout_duration, async {
            socksv5::v5::write_request_status(&mut stream, status.status, status.host, status.port)
                .await
        })
        .await
        .with_context(|| {
            format!("timeout sending {status_debug} response to {peer_address:?} {state:?}")
        })?
        .with_context(|| {
            format!("failed to send {status_debug} response to {peer_address:?} {state:?}")
        })?;

        if !is_success {
            return Ok(());
        }

        if matches!(state, ClientState::Connected(_)) {
            break;
        }
    }

    match state {
        ClientState::Connected(mut remote_stream) => {
            #[cfg(target_os = "linux")]
            if !no_splice {
                log::trace!("peer={peer_address:?} -> going to splice passthru mode");
                tokio_splice::zero_copy_bidirectional(&mut stream, &mut remote_stream).await?;
                return Ok(());
            }

            log::trace!("peer={peer_address:?} -> going to non-splice passthru mode");
            tokio::io::copy_bidirectional(&mut stream, &mut remote_stream).await?;
            Ok(())
        }
        _ => anyhow::bail!("Unexpected client state {state:?}"),
    }
}

/// Encapsulates the result of processing a command from the client
#[derive(Debug)]
struct RequestStatus {
    status: SocksV5RequestStatus,
    host: SocksV5Host,
    port: u16,
}

impl RequestStatus {
    /// Intended for failure cases, this method constructs a
    /// status with a zeroed-out host/port combination
    fn status(status: SocksV5RequestStatus) -> Self {
        Self {
            status,
            host: SocksV5Host::Ipv4([0, 0, 0, 0]),
            port: 0,
        }
    }

    /// Constructs a successful result, along with the address
    /// associated with it (typically the source address).
    fn success(addr: SocketAddr) -> Self {
        Self {
            status: SocksV5RequestStatus::Success,
            host: socket_addr_to_host(addr),
            port: addr.port(),
        }
    }

    /// Constructs an error based on the provided IoError
    fn error(err: std::io::Error) -> Self {
        Self::status(status_from_io_error(err))
    }

    fn timeout() -> Self {
        Self::status(SocksV5RequestStatus::TtlExpired)
    }
}

#[derive(Default, Debug)]
enum ClientState {
    #[default]
    None,
    Bound(TcpSocket),
    Connected(TcpStream),
}

async fn handle_request(
    request: &SocksV5Request,
    state: &mut ClientState,
) -> std::io::Result<RequestStatus> {
    match request.command {
        SocksV5Command::Bind => {
            if !matches!(state, ClientState::None) {
                return Ok(RequestStatus::status(SocksV5RequestStatus::ServerFailure));
            }

            let host = request_addr(&request).await?;
            let socket = match host {
                SocketAddr::V4(_) => TcpSocket::new_v4(),
                SocketAddr::V6(_) => TcpSocket::new_v6(),
            }?;

            socket.bind(host)?;
            let addr = socket.local_addr()?;

            *state = ClientState::Bound(socket);

            Ok(RequestStatus::success(addr))
        }
        SocksV5Command::Connect => {
            let host = request_addr(&request).await?;

            let addr = match std::mem::take(state) {
                ClientState::None => {
                    let stream = TcpStream::connect(host).await?;
                    let addr = stream.local_addr()?;
                    *state = ClientState::Connected(stream);
                    addr
                }
                ClientState::Bound(socket) => {
                    let stream = socket.connect(host).await?;
                    let addr = stream.local_addr()?;
                    *state = ClientState::Connected(stream);
                    addr
                }
                ClientState::Connected(_) => {
                    return Ok(RequestStatus::status(SocksV5RequestStatus::ServerFailure));
                }
            };

            Ok(RequestStatus::success(addr))
        }
        SocksV5Command::UdpAssociate => Ok(RequestStatus::status(
            SocksV5RequestStatus::CommandNotSupported,
        )),
    }
}

fn socket_addr_to_host(addr: SocketAddr) -> SocksV5Host {
    match addr {
        SocketAddr::V4(addr) => SocksV5Host::Ipv4(addr.ip().octets()),
        SocketAddr::V6(addr) => SocksV5Host::Ipv6(addr.ip().octets()),
    }
}

/// Maps an OS level error to a SOCKS5 status code
fn status_from_io_error(err: std::io::Error) -> SocksV5RequestStatus {
    match err.raw_os_error() {
        Some(libc::ENETUNREACH) => SocksV5RequestStatus::NetworkUnreachable,
        Some(libc::ETIMEDOUT) => SocksV5RequestStatus::TtlExpired,
        Some(libc::ECONNREFUSED) => SocksV5RequestStatus::ConnectionRefused,
        Some(libc::EHOSTUNREACH) => SocksV5RequestStatus::HostUnreachable,
        _ => SocksV5RequestStatus::ServerFailure,
    }
}

/// Convert a SocksV5Request host/port into a SocketAddr.
/// We fail for Domain requests as we don't support them
/// in the context of an MTA that is doing its own DNS.
async fn request_addr(request: &SocksV5Request) -> std::io::Result<SocketAddr> {
    match &request.host {
        SocksV5Host::Ipv4(ip) => Ok(SocketAddr::new(IpAddr::V4((*ip).into()), request.port)),
        SocksV5Host::Ipv6(ip) => Ok(SocketAddr::new(IpAddr::V6((*ip).into()), request.port)),
        SocksV5Host::Domain(_domain) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Domain not supported",
        )),
    }
}
