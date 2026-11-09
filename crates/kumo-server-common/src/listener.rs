//! Binding of TCP listeners with an operator-controllable accept queue.
//!
//! The async runtime builds listeners through mio, which hardcodes the
//! `listen(2)` backlog to 128 with no way to override it. That queue has been
//! observed to overflow under a connection burst well before CPU saturates,
//! dropping connections the kernel could otherwise have queued. This module
//! binds the socket directly so the backlog can be set, defaulting to the
//! kernel's ceiling.

use anyhow::Context;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{TcpListener, ToSocketAddrs};

/// Returns the kernel's maximum accept-queue length, which is the largest
/// backlog `listen(2)` will honor. A larger request is silently clamped down
/// to this value. On Linux this is `net.core.somaxconn`. Elsewhere, and when
/// it cannot be read, returns a conservative default that the kernel will
/// clamp down further if needed.
pub fn kernel_max_backlog() -> u32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/sys/net/core/somaxconn") {
            if let Ok(value) = contents.trim().parse::<u32>() {
                return value;
            }
        }
    }
    1024
}

/// Binds a listening TCP socket at `listen`, requesting an accept queue of
/// `backlog` entries, or the kernel maximum when `backlog` is `None`. The
/// returned listener is in non-blocking mode, ready to hand to an async
/// runtime.
///
/// A `backlog` larger than the kernel's ceiling is accepted here but silently
/// clamped by `listen(2)`.
pub fn bind_with_backlog(listen: &str, backlog: Option<u32>) -> anyhow::Result<TcpListener> {
    let backlog = backlog.unwrap_or_else(kernel_max_backlog);
    let backlog = backlog.min(i32::MAX as u32) as i32;

    let addrs = listen
        .to_socket_addrs()
        .with_context(|| format!("resolving {listen}"))?;

    let mut last_err = None;
    for addr in addrs {
        match bind_one(addr, backlog) {
            Ok(listener) => return Ok(listener),
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{listen} did not resolve to any address")))
}

fn bind_one(addr: std::net::SocketAddr, backlog: i32) -> anyhow::Result<TcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
        .context("creating socket")?;
    // Match the runtime's listeners, which set SO_REUSEADDR: without it, a
    // restart cannot rebind the address until the previous socket's TIME_WAIT
    // state expires.
    socket
        .set_reuse_address(true)
        .context("setting SO_REUSEADDR")?;
    socket
        .bind(&addr.into())
        .with_context(|| format!("binding to {addr}"))?;
    socket
        .listen(backlog)
        .with_context(|| format!("listening on {addr}"))?;
    socket
        .set_nonblocking(true)
        .context("setting nonblocking")?;
    Ok(socket.into())
}
