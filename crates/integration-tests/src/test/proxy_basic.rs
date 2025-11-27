use crate::proxy::{ProxyArgs, ProxyDaemon};

/// Test that the proxy server starts up successfully with no auth
#[tokio::test]
async fn proxy_basic_noauth() -> anyhow::Result<()> {
    let mut daemon = ProxyDaemon::spawn(ProxyArgs {
        policy_file: "proxy_init.lua".to_string(),
        env: vec![],
    })
    .await?;

    // Verify we got a listener
    let addr = daemon.listener("proxy");
    assert!(addr.port() > 0);

    daemon.stop().await?;
    Ok(())
}

/// Test that the proxy server starts up successfully with auth required
#[tokio::test]
async fn proxy_basic_with_auth() -> anyhow::Result<()> {
    let mut daemon = ProxyDaemon::spawn(ProxyArgs {
        policy_file: "proxy_auth.lua".to_string(),
        env: vec![],
    })
    .await?;

    // Verify we got a listener
    let addr = daemon.listener("proxy");
    assert!(addr.port() > 0);

    daemon.stop().await?;
    Ok(())
}
