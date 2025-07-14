use crate::kumod::{generate_message_text, DaemonWithMaildir, DeliverySummary, MailGenParams};
use kumo_log_types::RecordType;
use kumo_log_types::RecordType::TransientFailure;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use serial_test::serial;
use std::collections::BTreeMap;
use std::env;
use std::time::Duration;

// These test do not confirm the client side certificates are being sent to the server.
// They confirm the client side certificate paramters are being honored and loaded. Confirmed
// they're valid before proceeding with delivery. Failure in validation would result in temp failure..
// Add unit test for server side client side certificate handling once that feature is implemented.

#[tokio::test]
#[serial]
async fn tls_client_certificate_rustls_no_client_cert() -> anyhow::Result<()> {
    // Test with out defining certificate
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let r = tls_client_certificate(ex).await;
    return r;
}

#[tokio::test]
#[serial]
async fn tls_client_certificate_openssl_no_client_cert() -> anyhow::Result<()> {
    // Test with out defining certificate
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    env::set_var("PREFER_OPENSSL", "true");
    let r = tls_client_certificate(ex).await;
    env::remove_var("PREFER_OPENSSL");
    return r;
}

#[tokio::test]
#[serial]
async fn tls_client_certificate_rustls_success() -> anyhow::Result<()> {
    // Generate certificate, private key pair vua rcgen
    // rcgen supports creating x509v3 certificate.
    // Use rustls as TLS library and confirm delivery is successful
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    env::set_var("CLIENT_CERTIFICATE", &cert_pem);
    env::set_var("CLIENT_PRIVATE_KEY", &key_pem);
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let r = tls_client_certificate(ex).await;
    env::remove_var("CLIENT_CERTIFICATE");
    env::remove_var("CLIENT_PRIVATE_KEY");
    return r;
}

#[tokio::test]
#[serial]
async fn tls_client_certificate_rustls_fail() -> anyhow::Result<()> {
    // Adding fake private key to confirm rustls injection succeeds
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, .. } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    env::set_var("CLIENT_CERTIFICATE", &cert_pem);
    env::set_var("CLIENT_PRIVATE_KEY", "FAKE");
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([
            (RecordType::Reception, 1),
            (RecordType::TransientFailure, 1),
        ]),
        sink_counts: BTreeMap::new(),
    };
    let r = tls_client_certificate(ex).await;
    env::remove_var("CLIENT_CERTIFICATE");
    env::remove_var("CLIENT_PRIVATE_KEY");
    return r;
}

#[tokio::test]
#[serial]
async fn tls_client_certificate_rustls_openssl_success() -> anyhow::Result<()> {
    // Use openssl as TLS library, confirm delivery succeeds
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    env::set_var("PREFER_OPENSSL", "true");
    env::set_var("CLIENT_CERTIFICATE", &cert_pem);
    env::set_var("CLIENT_PRIVATE_KEY", &key_pem);
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let r = tls_client_certificate(ex).await;
    env::remove_var("PREFER_OPENSSL");
    env::remove_var("CLIENT_CERTIFICATE");
    env::remove_var("CLIENT_PRIVATE_KEY");
    return r;
}

#[tokio::test]
#[serial]
async fn tls_client_certificate_rustls_openssl_fail() -> anyhow::Result<()> {
    // Adding fake private key to confirm openssl injection would temp fail
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, .. } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    env::set_var("PREFER_OPENSSL", "true");
    env::set_var("CLIENT_CERTIFICATE", &cert_pem);
    env::set_var("CLIENT_PRIVATE_KEY", "FAKE");
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([
            (RecordType::Reception, 1),
            (RecordType::TransientFailure, 1),
        ]),
        sink_counts: BTreeMap::new(),
    };
    let r = tls_client_certificate(ex).await;
    env::remove_var("PREFER_OPENSSL");
    env::remove_var("CLIENT_CERTIFICATE");
    env::remove_var("CLIENT_PRIVATE_KEY");
    return r;
}

async fn tls_client_certificate(expected: DeliverySummary) -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(vec![
        // The default is OpportunisticInsecure; tighten it up a bit
        // so that we fail to deliver to the sink, and verify
        // the result of that attempt below
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
    ])
    .await?;

    let mut client = daemon.smtp_client().await?;

    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| summary.get(&TransientFailure).copied().unwrap_or(0) > 0,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await?;
    assert_eq!(delivery_summary, expected,);
    Ok(())
}
