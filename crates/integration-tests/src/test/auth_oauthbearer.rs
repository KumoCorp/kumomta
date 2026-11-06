use crate::kumod::{DaemonWithMaildir, KumoDaemon};
use data_encoding::BASE64;
use rfc5321::parser::Command;
use rfc5321::{Response, SmtpClient, TlsOptions, TlsStatus};

async fn tls_client(daemon: &KumoDaemon) -> anyhow::Result<SmtpClient> {
    let mut client = daemon.smtp_client("localhost").await?;
    let status = client
        .starttls(TlsOptions {
            insecure: true,
            ..Default::default()
        })
        .await?;
    anyhow::ensure!(matches!(status, TlsStatus::Info(_)), "{status:?}");
    client.ehlo("localhost").await?;
    Ok(client)
}

async fn auth_mechs(client: &mut SmtpClient) -> anyhow::Result<String> {
    let caps = client.ehlo("localhost").await?;
    Ok(caps
        .get("AUTH")
        .and_then(|cap| cap.param.clone())
        .unwrap_or_default())
}

async fn auth(
    client: &mut SmtpClient,
    sasl_mech: &str,
    initial_response: Option<&str>,
) -> anyhow::Result<Response> {
    Ok(client
        .send_command(&Command::Auth {
            sasl_mech: sasl_mech.to_string(),
            initial_response: initial_response.map(|payload| BASE64.encode(payload.as_bytes())),
        })
        .await?)
}

async fn send_line(client: &mut SmtpClient, line: &str) -> anyhow::Result<Response> {
    Ok(client.send_command(&Command::Unknown(line.into())).await?)
}

fn oauthbearer(authz: &str, token: &str) -> String {
    format!("n,{authz},\x01host=localhost\x01port=25\x01auth=Bearer {token}\x01\x01")
}

fn xoauth2(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

#[tokio::test]
async fn auth_oauthbearer() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start().await?;
    // Longer than the 998 byte command line limit
    let long_token = format!("valid-token{}", "x".repeat(4000));

    let mut client = tls_client(&daemon.sink).await?;
    k9::assert_equal!(auth_mechs(&mut client).await?, "PLAIN OAUTHBEARER XOAUTH2");
    let response = auth(
        &mut client,
        "OAUTHBEARER",
        Some(&oauthbearer("a=user@example.com", "valid-token")),
    )
    .await?;
    k9::assert_equal!(response.code, 235);

    let mut client = tls_client(&daemon.sink).await?;
    let response = auth(
        &mut client,
        "OAUTHBEARER",
        Some(&oauthbearer("", &long_token)),
    )
    .await?;
    k9::assert_equal!(response.code, 235);

    let mut client = tls_client(&daemon.sink).await?;
    let response = auth(&mut client, "XOAUTH2", None).await?;
    k9::assert_equal!(response.code, 334);
    let response = send_line(
        &mut client,
        &BASE64.encode(xoauth2("user@example.com", &long_token).as_bytes()),
    )
    .await?;
    k9::assert_equal!(response.code, 235);

    let mut client = tls_client(&daemon.sink).await?;
    let response = auth(
        &mut client,
        "OAUTHBEARER",
        Some(&oauthbearer("a=user@example.com", "bad-token")),
    )
    .await?;
    k9::assert_equal!(response.code, 334);
    k9::assert_equal!(
        BASE64.decode(response.content.as_bytes())?,
        br#"{"status":"invalid_token"}"#.to_vec()
    );
    let response = send_line(&mut client, "AQ==").await?;
    k9::assert_equal!(response.code, 535);

    let mut client = tls_client(&daemon.sink).await?;
    let response = auth(
        &mut client,
        "XOAUTH2",
        Some(&xoauth2("user@example.com", "bad-token")),
    )
    .await?;
    k9::assert_equal!(response.code, 334);
    let response = send_line(&mut client, "").await?;
    k9::assert_equal!(response.code, 535);

    let mut client = tls_client(&daemon.sink).await?;
    let response = auth(&mut client, "XOAUTH2", Some("user=user@example.com")).await?;
    k9::assert_equal!(response.code, 501);

    // source.lua defines no bearer token handlers
    let mut client = tls_client(&daemon.source).await?;
    k9::assert_equal!(auth_mechs(&mut client).await?, "PLAIN");
    let response = auth(
        &mut client,
        "OAUTHBEARER",
        Some(&oauthbearer("a=user@example.com", "valid-token")),
    )
    .await?;
    k9::assert_equal!(response.code, 504);

    let secrets = [
        "valid-token".to_string(),
        "bad-token".to_string(),
        BASE64.encode(oauthbearer("a=user@example.com", "bad-token").as_bytes()),
        BASE64.encode(xoauth2("user@example.com", "bad-token").as_bytes()),
        BASE64.encode(oauthbearer("a=user@example.com", "valid-token").as_bytes()),
    ];
    for daemon in [&daemon.source, &daemon.sink] {
        for record in daemon.collect_logs().await? {
            let json = serde_json::to_string(&record)?;
            for secret in &secrets {
                anyhow::ensure!(
                    !json.contains(secret.as_str()),
                    "token leaked into a log record: {json}"
                );
            }
        }
    }

    daemon.stop_both().await?;
    Ok(())
}
