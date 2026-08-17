use crate::kumod::DaemonWithMaildir;
use k9::assert_equal;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// A line whose bytes are not valid UTF-8 should be rejected with an SMTP
/// syntax error, and the session should remain usable for subsequent
/// commands, rather than tearing the connection down with a 421.
#[tokio::test]
async fn invalid_utf8_command_is_rejected() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    let addr = daemon.source.listener("smtp");

    let mut stream = TcpStream::connect(addr).await?;
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    // Consume the banner
    let mut banner = String::new();
    reader.read_line(&mut banner).await?;
    anyhow::ensure!(banner.starts_with("220 "), "unexpected banner: {banner:?}");

    // Bytes in the range 0x80-0xFF are never valid on their own in UTF-8;
    // terminated by CRLF they form a "line" that cannot be decoded.
    writer.write_all(b"\x80\x81\x82\x83\r\n").await?;

    let rejection = read_line_lossy(&mut reader).await?;
    assert_equal!(rejection, "501 5.5.2 Invalid UTF-8 in command\r\n");

    // The session must survive; a valid command still works.
    writer.write_all(b"NOOP\r\n").await?;
    let noop = read_line_lossy(&mut reader).await?;
    assert_equal!(noop, "250 the goggles do nothing\r\n");

    writer.write_all(b"QUIT\r\n").await?;

    daemon.stop_both().await?;
    Ok(())
}

/// Read a single CRLF-terminated response line without assuming the bytes
/// are valid UTF-8, so that a regression which echoes raw input back cannot
/// trip up the test harness itself.
async fn read_line_lossy<R>(reader: &mut R) -> anyhow::Result<String>
where
    R: AsyncReadExt + Unpin,
{
    let mut buf = vec![];
    loop {
        let mut byte = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), reader.read(&mut byte)).await??;
        anyhow::ensure!(n == 1, "unexpected EOF while reading response");
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
