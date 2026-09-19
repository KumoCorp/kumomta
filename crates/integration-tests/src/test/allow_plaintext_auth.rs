use crate::kumod::{KumoDaemon, KumoArgs};

async fn spawn(allow_plaintext: bool) -> anyhow::Result<KumoDaemon> {
    let mut env = vec![];
    if allow_plaintext {
        env.push(("KUMOD_ALLOW_PLAINTEXT_AUTH".to_string(), "true".to_string()));
    }
    KumoDaemon::spawn(KumoArgs {
        policy_file: "plaintext-auth.lua".to_string(),
        env,
    })
    .await
}

/// Without allow_plaintext_auth, STARTTLS is advertised but AUTH is not.
#[tokio::test]
async fn auth_not_advertised_before_tls_by_default() -> anyhow::Result<()> {
    let mut daemon = spawn(false).await?;
    let mut client = daemon.smtp_client("localhost").await?;

    let caps = client.ehlo_lhlo("there", false).await?;
    assert!(
        caps.contains_key("STARTTLS"),
        "STARTTLS should be advertised before TLS"
    );
    assert!(
        !caps.contains_key("AUTH"),
        "AUTH should not be advertised before TLS when allow_plaintext_auth is false"
    );

    daemon.stop().await?;
    Ok(())
}

/// With allow_plaintext_auth, AUTH is advertised even before STARTTLS.
#[tokio::test]
async fn auth_advertised_before_tls_when_plaintext_auth_enabled() -> anyhow::Result<()> {
    let mut daemon = spawn(true).await?;
    let mut client = daemon.smtp_client("localhost").await?;

    let caps = client.ehlo_lhlo("there", false).await?;
    assert!(
        caps.contains_key("AUTH"),
        "AUTH should be advertised when allow_plaintext_auth is true"
    );
    assert!(
        !caps.contains_key("STARTTLS"),
        "STARTTLS should not be advertised when allow_plaintext_auth is true"
    );

    daemon.stop().await?;
    Ok(())
}

/// With allow_plaintext_auth, AUTH PLAIN actually succeeds without TLS.
#[tokio::test]
async fn auth_succeeds_without_tls_when_plaintext_auth_enabled() -> anyhow::Result<()> {
    let mut daemon = spawn(true).await?;
    let mut client = daemon.smtp_client("localhost").await?;

    client.auth_plain("testuser", Some("testpass")).await?;

    daemon.stop().await?;
    Ok(())
}
