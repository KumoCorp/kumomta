use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

pub trait AsyncReadAndWrite: AsyncRead + AsyncWrite + Debug + Unpin + Send {}
impl AsyncReadAndWrite for TlsStream<TcpStream> {}
impl AsyncReadAndWrite for TcpStream {}

pub type BoxedAsyncReadAndWrite = Box<dyn AsyncReadAndWrite>;
