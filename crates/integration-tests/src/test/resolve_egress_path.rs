use crate::kumod::DaemonWithMaildirOptions;
use anyhow::Context;
use kumo_api_types::egress_path::CeilingSource;
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

    // Second domain: the policy declares
    //   path.max_message_rate    = 1000/s
    //   queue.max_message_rate   = 100/s
    // Folding the queue constraint into the path constraints should
    // produce a message-rate ceiling of 100/s sourced from the
    // scheduled queue, with 1000/s recorded as "declared but
    // unreachable".
    let resp = api
        .admin_resolve_egress_path_v1(&ResolveEgressPathV1Request {
            domain: "sched-rate.example.com".to_string(),
            source: Some("my-source".to_string()),
        })
        .await
        .context("admin_resolve_egress_path_v1 sched-rate")?;

    let mr = resp
        .constraints
        .max_message_rate
        .as_ref()
        .context("max_message_rate present")?;
    assert_eq!(mr.value, 100.0);
    assert_eq!(
        mr.source,
        CeilingSource::Other {
            name: "scheduled queue max_message_rate".to_string()
        }
    );
    assert_eq!(
        resp.constraints.max_message_rate_declared.as_deref(),
        Some("1000/s")
    );
    // The full queue_config is shipped on the wire as JSON; verify
    // the rate appears there too so callers can inspect it.
    let qc_rate = resp
        .queue_config
        .get("max_message_rate")
        .context("queue_config.max_message_rate present")?;
    assert_eq!(qc_rate, "100/s");

    daemon.stop_both().await.context("stop_both")?;
    Ok(())
}
