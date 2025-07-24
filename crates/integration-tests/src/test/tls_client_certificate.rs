use crate::kumod::{DaemonWithMaildir, DeliverySummary, MailGenParams};
use k9::assert_equal;
use kumo_log_types::RecordType;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use std::collections::BTreeMap;
use std::time::Duration;

// These test do not confirm the client side certificates are being sent to the server.
// They confirm the client side certificate paramters are being honored and loaded. Confirmed
// they're valid before proceeding with delivery. Failure in validation would result in temp failure..
// Add unit test for server side client side certificate handling once that feature is implemented.

#[tokio::test]
async fn tls_client_certificate_rustls_no_client_cert() -> anyhow::Result<()> {
    // Test with out defining certificate
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let env = vec![("KUMOD_ENABLE_TLS", "OpportunisticInsecure")];
    tls_client_certificate(env, ex).await
}

#[tokio::test]
async fn tls_client_certificate_openssl_no_client_cert() -> anyhow::Result<()> {
    // Test with out defining certificate
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "true"),
    ];
    tls_client_certificate(env, ex).await
}

#[tokio::test]
async fn tls_client_certificate_rustls_success() -> anyhow::Result<()> {
    // Generate certificate, private key pair vua rcgen
    // rcgen supports creating x509v3 certificate.
    // Use rustls as TLS library and confirm delivery is successful
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "true"),
        ("KUMOD_CLIENT_CERTIFICATE", &cert_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", &key_pem),
    ];
    tls_client_certificate(env, ex).await
}

#[tokio::test]
async fn tls_client_certificate_rustls_fail() -> anyhow::Result<()> {
    // Adding fake private key to confirm rustls injection succeeds
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, .. } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([
            (RecordType::Reception, 1),
            (RecordType::TransientFailure, 1),
        ]),
        sink_counts: BTreeMap::new(),
    };
    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_CLIENT_CERTIFICATE", &cert_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", "FAKE"),
    ];
    tls_client_certificate(env, ex).await
}

#[tokio::test]
async fn tls_client_certificate_rustls_openssl_success() -> anyhow::Result<()> {
    // Use openssl as TLS library, confirm delivery succeeds
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "true"),
        ("KUMOD_CLIENT_CERTIFICATE", &cert_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", &key_pem),
    ];
    tls_client_certificate(env, ex).await
}

#[tokio::test]
async fn tls_client_certificate_rustls_openssl_fail() -> anyhow::Result<()> {
    // Adding fake private key to confirm openssl injection would temp fail
    let subject_alt_names = vec!["smtp.example.com".to_string()];
    let CertifiedKey { cert, .. } = generate_simple_self_signed(subject_alt_names)?;
    let cert_pem = cert.pem();
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([
            (RecordType::Reception, 1),
            (RecordType::TransientFailure, 1),
        ]),
        sink_counts: BTreeMap::new(),
    };
    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "true"),
        ("KUMOD_CLIENT_CERTIFICATE", &cert_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", "FAKE"),
    ];
    tls_client_certificate(env, ex).await
}

async fn tls_client_certificate(
    env: Vec<(&str, &str)>,
    expected: DeliverySummary,
) -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildir::start_with_env(env).await?;
    let mut client = daemon.smtp_client().await?;

    let response = MailGenParams::default().send(&mut client).await?;
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_source_summary(
            |summary| *summary == expected.source_counts,
            Duration::from_secs(50),
        )
        .await;

    daemon.stop_both().await?;
    println!("Stopped!");

    let delivery_summary = daemon.dump_logs().await?;
    assert_equal!(delivery_summary, expected);

    if expected.sink_counts.contains_key(&RecordType::Delivery) {
        let sink_logs = daemon.sink.collect_logs().await?;
        let reception = sink_logs
            .iter()
            .find(|record| record.kind == RecordType::Reception)
            .unwrap();
        eprintln!("sink: {reception:#?}");
        assert!(!reception.tls_protocol_version.as_ref().unwrap().is_empty());
        assert!(!reception.tls_cipher.as_ref().unwrap().is_empty());
    }
    Ok(())
}
