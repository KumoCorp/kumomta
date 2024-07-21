use std::fmt::Debug;
use std::os::fd::{AsRawFd, FromRawFd};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_openssl::SslStream;
use tokio_rustls::client::TlsStream as TlsClientStream;
use tokio_rustls::server::TlsStream as TlsServerStream;

pub trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Debug + Unpin + Send {
    /// Optionally clone a TcpStream that represents the same underlying
    /// stream as this one.
    /// This only has an impl that returns Some for TcpStream.
    /// It is present to facilitate a workaround for some awkwardness
    /// in the SslStream implementation for the failed-handshake case.
    fn try_dup(&self) -> Option<TcpStream> {
        None
    }
}
impl AsyncReadAndWrite for TlsClientStream<TcpStream> {}
impl AsyncReadAndWrite for TlsClientStream<BoxedAsyncReadAndWrite> {}
impl AsyncReadAndWrite for TlsServerStream<TcpStream> {}
impl AsyncReadAndWrite for TlsServerStream<BoxedAsyncReadAndWrite> {}

impl AsyncReadAndWrite for TcpStream {
    fn try_dup(&self) -> Option<TcpStream> {
        let fd = self.as_raw_fd();
        // SAFETY: dup creates a new fd without affecting the state
        // of other descriptors
        let duplicate = unsafe { libc::dup(fd) };
        if duplicate == -1 {
            None
        } else {
            // SAFETY: we're wrapping the new duplicate from above,
            // which is fine, and provides a destructor for that fd
            // when the TcpStream is dropped
            let duplicate_stream = unsafe { std::net::TcpStream::from_raw_fd(duplicate) };
            TcpStream::from_std(duplicate_stream).ok()
        }
    }
}
impl AsyncReadAndWrite for SslStream<TcpStream> {}
impl AsyncReadAndWrite for SslStream<BoxedAsyncReadAndWrite> {}

pub type BoxedAsyncReadAndWrite = Box<dyn AsyncReadAndWrite>;
