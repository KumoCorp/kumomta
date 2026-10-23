use crate::kumod::{DaemonWithMaildir, MailGenParams};
use bstr::ByteSlice;
use k9::assert_equal;
use kumo_log_types::RecordType::{Delivery, Reception};
use serde_json::Value;
use std::time::Duration;

/// Verifies end-to-end delivery over implicit TLS (SMTPS): the sink listener
/// and the source egress path both run with `tls_mode = ImplicitTls` instead of
/// STARTTLS. The sink presents a self-signed certificate. The source keeps the
/// harness's `OpportunisticInsecure` `enable_tls` (source.lua) so that
/// certificate is accepted without validation.
#[tokio::test]
async fn tls_implicit() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        ("KUMOD_TEST_SINK_TLS_MODE", "ImplicitTls"),
        ("KUMOD_EGRESS_TLS_MODE", "ImplicitTls"),
    ])
    .await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&Delivery).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;

    // The source delivered over an encrypted channel.
    let source_logs = daemon.source.collect_logs().await?;
    let delivery = source_logs
        .iter()
        .find(|record| record.kind == Delivery)
        .unwrap();
    assert!(delivery.tls_cipher.is_some());

    // Confirm the sink recognized the session as implicit TLS rather than as
    // a plaintext connection that happened to upgrade via STARTTLS.
    let sink_logs = daemon.sink.collect_logs().await?;
    let reception = sink_logs
        .iter()
        .find(|record| record.kind == Reception)
        .unwrap();
    assert_equal!(delivery.tls_cipher, reception.tls_cipher);
    assert_equal!(reception.meta.get("tls_implicit"), Some(&Value::Bool(true)));

    // The received message is traced as ESMTPS, the same as a STARTTLS session.
    let mut messages = daemon.extract_maildir_messages()?;
    assert_equal!(messages.len(), 1);
    let parsed = messages[0].parsed()?;
    let trace = parsed
        .headers()
        .get_first("Received")
        .unwrap()
        .as_unstructured()
        .unwrap();
    assert!(trace.contains_str(&format!(
        "with ESMTPS ({}:{})",
        reception.tls_protocol_version.as_ref().unwrap(),
        reception.tls_cipher.as_ref().unwrap()
    )));

    Ok(())
}
