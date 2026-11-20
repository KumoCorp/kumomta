use crate::kumod::{KumoArgs, KumoDaemon};
use anyhow::Context;
use rfc5321::parser::Command;
use rfc5321::{ClientError, SmtpClient, TlsOptions};

/// Start a sink daemon, optionally with `enable_auth_login` on its listener
async fn start(enable_auth_login: bool) -> anyhow::Result<KumoDaemon> {
    let env = if enable_auth_login {
        vec![("KUMOD_ENABLE_AUTH_LOGIN".to_string(), "1".to_string())]
    } else {
        vec![]
    };

    KumoDaemon::spawn(KumoArgs {
        policy_file: "sink.lua".to_string(),
        env,
    })
    .await
    .context("spawn sink")
}

/// Connect and STARTTLS, returning the AUTH capability advertised on the
/// encrypted channel
async fn connect(daemon: &KumoDaemon) -> anyhow::Result<(SmtpClient, Option<String>)> {
    let mut client = daemon.smtp_client("localhost").await?;

    // The sink presents a self-signed certificate
    client
        .starttls(TlsOptions {
            insecure: true,
            ..Default::default()
        })
        .await
        .context("starttls")?;

    let capabilities = client.ehlo("localhost").await.context("ehlo")?;
    let auth = capabilities.get("AUTH").and_then(|cap| cap.param.clone());

    Ok((client, auth))
}

#[tokio::test]
async fn auth_login_ok() -> anyhow::Result<()> {
    let mut daemon = start(true).await?;
    let (mut client, auth) = connect(&daemon).await?;

    k9::assert_equal!(auth.as_deref(), Some("PLAIN LOGIN"));

    client
        .auth_login("daniel", Some("tiger"))
        .await
        .context("auth_login")?;

    daemon.stop().await?;
    Ok(())
}

/// A username passed as the initial-response leaves only the password to
/// challenge for
#[tokio::test]
async fn auth_login_initial_response() -> anyhow::Result<()> {
    let mut daemon = start(true).await?;
    let (mut client, _auth) = connect(&daemon).await?;

    let response = client
        .send_command(&Command::Auth {
            sasl_mech: "LOGIN".to_string(),
            // base64("daniel")
            initial_response: Some("ZGFuaWVs".to_string()),
        })
        .await?;
    k9::assert_equal!(response.code, 334);
    // base64("Password:")
    k9::assert_equal!(response.content.trim(), "UGFzc3dvcmQ6");

    // base64("tiger")
    let response = client
        .send_command(&Command::Unknown("dGlnZXI=".into()))
        .await?;
    k9::assert_equal!(response.code, 235);

    daemon.stop().await?;
    Ok(())
}

#[tokio::test]
async fn auth_login_invalid_password() -> anyhow::Result<()> {
    let mut daemon = start(true).await?;
    let (mut client, _auth) = connect(&daemon).await?;

    match client.auth_login("daniel", Some("not-tiger")).await {
        Err(ClientError::Rejected(response)) => {
            k9::assert_equal!(response.code, 535);
        }
        result => anyhow::bail!("expected a 535 rejection, got {result:?}"),
    }

    daemon.stop().await?;
    Ok(())
}

/// `*` cancels the exchange rather than reading as a credential
#[tokio::test]
async fn auth_login_cancelled() -> anyhow::Result<()> {
    let mut daemon = start(true).await?;
    let (mut client, _auth) = connect(&daemon).await?;

    let response = client
        .send_command(&Command::Auth {
            sasl_mech: "LOGIN".to_string(),
            initial_response: None,
        })
        .await?;
    k9::assert_equal!(response.code, 334);
    // base64("Username:")
    k9::assert_equal!(response.content.trim(), "VXNlcm5hbWU6");

    let response = client.send_command(&Command::Unknown("*".into())).await?;
    k9::assert_equal!(response.code, 501);

    daemon.stop().await?;
    Ok(())
}

/// Without the opt-in, LOGIN is neither advertised nor accepted, and PLAIN
/// still works
#[tokio::test]
async fn auth_login_not_enabled() -> anyhow::Result<()> {
    let mut daemon = start(false).await?;
    let (mut client, auth) = connect(&daemon).await?;

    k9::assert_equal!(auth.as_deref(), Some("PLAIN"));

    let response = client
        .send_command(&Command::Auth {
            sasl_mech: "LOGIN".to_string(),
            initial_response: None,
        })
        .await?;
    k9::assert_equal!(response.code, 504);

    client
        .auth_plain("daniel", Some("tiger"))
        .await
        .context("auth_plain")?;

    daemon.stop().await?;
    Ok(())
}

/// LOGIN is refused on an unencrypted channel, just like PLAIN
#[tokio::test]
async fn auth_login_requires_tls() -> anyhow::Result<()> {
    let mut daemon = start(true).await?;
    let mut client = daemon.smtp_client("localhost").await?;

    let response = client
        .send_command(&Command::Auth {
            sasl_mech: "LOGIN".to_string(),
            initial_response: None,
        })
        .await?;
    k9::assert_equal!(response.code, 524);

    daemon.stop().await?;
    Ok(())
}
