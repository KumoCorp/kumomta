use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use kumo_api_types::ResolveEgressPathV1Request;

#[tokio::test]
async fn resolve_egress_path() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("source-inspect.lua")
        .start()
        .await
        .context("DaemonWithMaildirOptions::start")?;

    let api = daemon.source.api_client();

    // Use a domain that the policy treats as a Lua-protocol custom
    // route (source-inspect.lua routes every domain through
    // make.slow_send). DNS resolution is not required for this; the
    // protocol path uses the routing_domain directly.
    let resp = api
        .admin_resolve_egress_path_v1(&ResolveEgressPathV1Request {
            domain: "inspect.example.com".to_string(),
            source: Some("my-source".to_string()),
        })
        .await
        .context("admin_resolve_egress_path_v1")?;

    anyhow::ensure!(resp.domain == "inspect.example.com");
    anyhow::ensure!(resp.source == "my-source");
    anyhow::ensure!(
        resp.queue_name == "my-source->inspect.example.com@lua:make.slow_send",
        "unexpected queue_name: {}",
        resp.queue_name
    );
    // Lua-protocol path: MX not used, so we expect None here.
    anyhow::ensure!(resp.mx.is_none(), "expected mx=None; got {:?}", resp.mx);
    // The path_config has the same overrides set by source-inspect.lua.
    anyhow::ensure!(resp.path_config.connection_limit.limit == 1);
    // The synthetic reconnect-cycling rate ceiling is not in play
    // because no max_connection_rate is set, so the message-rate
    // ceiling should be None.
    anyhow::ensure!(resp.constraints.max_message_rate.is_none());
    // Concurrency ceiling must be the configured connection_limit=1.
    anyhow::ensure!(
        resp.constraints.max_concurrent_dispatchers.value == 1.0,
        "unexpected concurrency ceiling: {:?}",
        resp.constraints.max_concurrent_dispatchers
    );

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}
