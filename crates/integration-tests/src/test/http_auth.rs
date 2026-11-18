use crate::kumod::DaemonWithMaildir;

#[tokio::test]
async fn http_auth() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;

    let client = kumo_http_client::builder().build().unwrap();
    let response = client
        .get(&format!("http://{}/metrics", daemon.sink.listener("http")))
        .send()
        .await?;

    k9::assert_equal!(
        response.status(),
        200,
        "Response status: {}",
        response.status()
    );

    // The username and password here are hard-coded into maildir-sink.lua
    // for the integration test suite

    let response = client
        .get(&format!(
            "http://daniel:tiger@{}/metrics",
            daemon.sink.listener("http")
        ))
        .send()
        .await?;

    k9::assert_equal!(
        response.status(),
        200,
        "Response status: {}",
        response.status()
    );

    let response = client
        .get(&format!(
            "http://daniel:bogus@{}/metrics",
            daemon.sink.listener("http")
        ))
        .send()
        .await?;

    k9::assert_equal!(
        response.status(),
        401,
        "Response status: {}",
        response.status()
    );

    daemon.stop_both().await?;

    Ok(())
}
