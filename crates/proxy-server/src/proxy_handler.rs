#[cfg(target_os = "linux")]
use crate::splice_copy::splice_copy as copy_stream;
use anyhow::Context;
use socksv5::v5::{
    SocksV5AuthMethod, SocksV5Command, SocksV5Host, SocksV5Request, SocksV5RequestStatus,
};
use std::net::{IpAddr, SocketAddr};
use tokio::io::AsyncWriteExt;
#[cfg(not(target_os = "linux"))]
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::timeout;

/// Given a newly accepted client, perform a SOCKS5 handshake and then
/// run the proxy logic, passing data from the client through to the
/// host to which we have connected on the behalf of the client.
pub async fn handle_proxy_client(
    mut stream: TcpStream,
    peer_address: SocketAddr,
    timeout_duration: std::time::Duration,
) -> anyhow::Result<()> {
    let (mut src_reader, mut src_writer) = stream.split();
    let mut state = ClientState::None;

    let handshake = timeout(timeout_duration, async {
        socksv5::v5::read_handshake(&mut src_reader)
            .await
            .context("reading client handshake")
    })
    .await??;

    if handshake
        .methods
        .into_iter()
        .find(|m| *m == socksv5::v5::SocksV5AuthMethod::Noauth)
        .is_none()
    {
        return Err(anyhow::anyhow!("this proxy only supports NOAUTH"));
    }

    timeout(timeout_duration, async {
        socksv5::v5::write_auth_method(&mut src_writer, SocksV5AuthMethod::Noauth).await
    })
    .await??;

    loop {
        let request = timeout(timeout_duration, async {
            socksv5::v5::read_request(&mut src_reader).await
        })
        .await??;
        log::trace!("peer={peer_address:?} request: {request:?}");

        let status = match timeout(timeout_duration, async {
            handle_request(request, &mut state).await
        })
        .await?
        {
            Ok(s) => s,
            Err(err) => RequestStatus::error(err),
        };

        let is_success = status.status == SocksV5RequestStatus::Success;
        log::trace!("peer={peer_address:?}: status -> {status:?}");

        timeout(timeout_duration, async {
            socksv5::v5::write_request_status(
                &mut src_writer,
                status.status,
                status.host,
                status.port,
            )
            .await
        })
        .await??;

        if !is_success {
            return Ok(());
        }

        if matches!(state, ClientState::Connected(_)) {
            break;
        }
    }

    match state {
        ClientState::Connected(mut stream) => {
            log::trace!("peer={peer_address:?} -> going to passthru mode");

            let (mut dest_reader, mut dest_writer) = stream.split();

            let src_to_dest = async {
                copy_stream(&mut src_reader, &mut dest_writer).await?;
                dest_writer.shutdown().await?;
                Ok(())
            };

            let dest_to_src = async {
                copy_stream(&mut dest_reader, &mut src_writer).await?;
                src_writer.shutdown().await?;
                Ok(())
            };

            tokio::try_join!(src_to_dest, dest_to_src).map(|_| ())
        }
        _ => anyhow::bail!("Unexpected client state {state:?}"),
    }
}

#[cfg(not(target_os = "linux"))]
async fn copy_stream(src: &mut ReadHalf<'_>, dst: &mut WriteHalf<'_>) -> anyhow::Result<()> {
    tokio::io::copy(src, dst).await?;
    Ok(())
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
}

#[derive(Default, Debug)]
enum ClientState {
    #[default]
    None,
    Bound(TcpSocket),
    Connected(TcpStream),
}

async fn handle_request(
    request: SocksV5Request,
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
