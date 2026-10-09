//! Async stream traits for TLS and plain TCP connections.

use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_openssl::SslStream;
use tokio_rustls::client::TlsStream as TlsClientStream;
use tokio_rustls::server::TlsStream as TlsServerStream;

pub trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Debug + Unpin + Send + Sync {
    /// Returns Ok() if the type can be converted without loss to a TcpStream,
    /// or Err(self) otherwise.  This is used by the proxy server to decide
    /// whether we can use splice(2).
    fn try_into_tcp_stream(self) -> Result<TcpStream, Self>
    where
        Self: Sized,
    {
        Err(self)
    }
}
impl AsyncReadAndWrite for TlsClientStream<TcpStream> {}
impl AsyncReadAndWrite for TlsClientStream<BoxedAsyncReadAndWrite> {}
impl AsyncReadAndWrite for TlsServerStream<TcpStream> {}
impl AsyncReadAndWrite for TlsServerStream<BoxedAsyncReadAndWrite> {}

impl AsyncReadAndWrite for TcpStream {
    // Yes, a TcpStream can be converted to a TcpStream
    fn try_into_tcp_stream(self) -> Result<TcpStream, Self> {
        Ok(self)
    }
}
impl AsyncReadAndWrite for SslStream<TcpStream> {}
impl AsyncReadAndWrite for SslStream<BoxedAsyncReadAndWrite> {}
impl AsyncReadAndWrite for tokio::net::UnixStream {}

pub type BoxedAsyncReadAndWrite = Box<dyn AsyncReadAndWrite>;

impl AsyncReadAndWrite for BoxedAsyncReadAndWrite {}
