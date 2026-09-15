use crate::kumod::{generate_message_text, DaemonWithTsa, MailGenParams};
use kumo_log_types::RecordType::TransientFailure;
use std::time::Duration;

// Confirms AdjustDomainConfig forces mx_rollup=false while AdjustConfig
// follows the enclosing rule's scope. Uses two "default"-scoped rules,
// since the rollup distinction is only observable at that scope (a
// domain-specific rule always has was_rollup=false regardless of action).
// Different decrease_percent values (50 vs 25) give distinguishable
// connection_limit results (16 vs 24), so each lookup can unambiguously
// identify which entry it found.
#[tokio::test]
async fn tsa_adjust_config_vs_domain_config_rollup() -> anyhow::Result<()> {
    let mut daemon = DaemonWithTsa::start().await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-default-rollup@rollup1.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        recip: Some("450-adjust-default-norollup@norollup1.mx-sink.wezfurlong.org"),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .with_maildir
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) >= 2,
            Duration::from_secs(50),
        )
        .await;

    const SITE: &str = "loopback.dummy-mx.wezfurlong.org";
    const BOGUS_SITE: &str = "totally-bogus-site.invalid";
    const BOGUS_DOMAIN: &str = "totally-bogus-domain.invalid";

    async fn connection_limit_at(
        daemon: &DaemonWithTsa,
        domain: &str,
        site_name: &str,
    ) -> anyhow::Result<Option<i64>> {
        let shaping = daemon.tsa.get_shaping().await?;
        let partial = shaping
            .get_egress_path_config_value(domain, "unspecified", site_name)
            .await?;
        Ok(partial.get("connection_limit").and_then(|v| v.as_i64()))
    }

    // Poll until both entries have shown up somewhere, then take a final
    // reading of all four lookups for the assertions below.
    for _ in 0..20 {
        let rollup = connection_limit_at(&daemon, BOGUS_DOMAIN, SITE).await?;
        let norollup =
            connection_limit_at(&daemon, "norollup1.mx-sink.wezfurlong.org", BOGUS_SITE).await?;
        if rollup.is_some() && norollup.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // AdjustConfig at "default" scope: was_rollup=true, base 32,
    // decrease_percent=50 -> 16. Found via the resolved site, not the raw
    // domain, proving it was stored under by_site rather than by_domain.
    assert_eq!(
        connection_limit_at(&daemon, BOGUS_DOMAIN, SITE).await?,
        Some(16),
        "AdjustConfig override should be found via the resolved site name"
    );
    assert_eq!(
        connection_limit_at(&daemon, "rollup1.mx-sink.wezfurlong.org", BOGUS_SITE).await?,
        None,
        "AdjustConfig override should NOT be found via domain + wrong site"
    );

    // AdjustDomainConfig: mx_rollup forced to false regardless of scope,
    // base 32, decrease_percent=25 -> 24. Found via the raw recipient
    // domain, proving it was stored under by_domain rather than by_site.
    assert_eq!(
        connection_limit_at(&daemon, "norollup1.mx-sink.wezfurlong.org", BOGUS_SITE).await?,
        Some(24),
        "AdjustDomainConfig override should be found via the raw domain"
    );

    daemon.stop().await?;
    Ok(())
}
