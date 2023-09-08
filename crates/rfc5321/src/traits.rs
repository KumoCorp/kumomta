use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_openssl::SslStream;
use tokio_rustls::client::TlsStream as TlsClientStream;
use tokio_rustls::server::TlsStream as TlsServerStream;

pub trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Debug + Unpin + Send {}
impl AsyncReadAndWrite for TlsClientStream<TcpStream> {}
impl AsyncReadAndWrite for TlsClientStream<BoxedAsyncReadAndWrite> {}
impl AsyncReadAndWrite for TlsServerStream<TcpStream> {}
impl AsyncReadAndWrite for TlsServerStream<BoxedAsyncReadAndWrite> {}
impl AsyncReadAndWrite for TcpStream {}
impl AsyncReadAndWrite for SslStream<TcpStream> {}
impl AsyncReadAndWrite for SslStream<BoxedAsyncReadAndWrite> {}

pub type BoxedAsyncReadAndWrite = Box<dyn AsyncReadAndWrite>;
