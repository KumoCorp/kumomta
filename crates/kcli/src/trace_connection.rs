use kumo_api_types::TraceDetail;
use reqwest::Url;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub async fn connect(
    endpoint: &Url,
    path: &str,
    requested: Option<TraceDetail>,
) -> anyhow::Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let mut url = endpoint.join(path)?;
    url.set_scheme(if endpoint.scheme() == "https" {
        "wss"
    } else {
        "ws"
    })
    .expect("valid WebSocket scheme");
    if let Some(detail) = requested {
        url.query_pairs_mut()
            .append_pair("detail", &detail.to_string());
    }
    let (socket, response) = connect_async(url.as_str()).await.map_err(|err| {
        if let tokio_tungstenite::tungstenite::Error::Http(res) = &err {
            if let Some(body) = res.body() {
                return anyhow::anyhow!("{err}: {}", String::from_utf8_lossy(body));
            }
        }
        anyhow::Error::from(err)
    })?;
    let Some(header) = response.headers().get("kumo-trace-detail") else {
        anyhow::ensure!(
            requested.is_none(),
            "server does not negotiate SMTP trace detail"
        );
        eprintln!("Legacy SMTP trace: full protocol including credentials");
        return Ok(socket);
    };
    let detail: TraceDetail = header.to_str()?.parse()?;
    anyhow::ensure!(
        requested.is_none_or(|r| r == detail),
        "server returned unexpected trace detail: {detail}"
    );
    eprintln!("Effective SMTP trace detail: {detail}");
    Ok(socket)
}
