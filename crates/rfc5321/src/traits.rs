use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

pub trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl AsyncReadAndWrite for TlsStream<TcpStream> {}
impl AsyncReadAndWrite for TcpStream {}
