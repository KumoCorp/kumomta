use crate::kumod::{DaemonWithMaildir, DeliverySummary, MailGenParams};
use k9::assert_equal;
use kumo_log_types::RecordType;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use std::collections::BTreeMap;
use std::time::Duration;

// These test do not confirm the client side certificates are being sent to the server.
// They confirm the client side certificate paramters are being honored and loaded. Confirmed
// they're valid before proceeding with delivery. Failure in validation would result in temp failure..

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

/// Generate certificate, private key pair vua rcgen
/// rcgen supports creating x509v3 certificate.
/// Use rustls as TLS library and confirm delivery is successful
#[tokio::test]
async fn tls_client_certificate_rustls_success() -> anyhow::Result<()> {
    let (ca_pem, intermediate_pem, entity_pem, key_pem) = generate_certs()?;
    let cert_pem = format!("{entity_pem}\n{intermediate_pem}");
    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };
    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "false"),
        ("KUMOD_CLIENT_CERTIFICATE", &cert_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", &key_pem),
        ("KUMOD_CLIENT_REQUIRED_CA", &ca_pem),
    ];
    tls_client_certificate(env, ex).await
}

/// Adding fake private key to confirm rustls injection succeeds
#[tokio::test]
async fn tls_client_certificate_rustls_fail() -> anyhow::Result<()> {
    let (_ca_pem, _intermediate_pem, cert_pem, _key_pem) = generate_certs()?;
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

const COMMON_NAME: &str = "Testing Common Name";

fn generate_certs() -> anyhow::Result<(String, String, String, String)> {
    // Root CA
    let mut root_params = CertificateParams::default();
    root_params.distinguished_name = DistinguishedName::new();
    root_params
        .distinguished_name
        .push(DnType::CountryName, "GB");
    root_params
        .distinguished_name
        .push(DnType::OrganizationName, "kumo-testing");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    let root_key = KeyPair::generate()?;
    let root_ca = CertifiedIssuer::self_signed(root_params, root_key)?;

    // Intermediate CA (signed by root)
    let mut intermediate_params = CertificateParams::default();
    intermediate_params.distinguished_name = DistinguishedName::new();
    intermediate_params
        .distinguished_name
        .push(DnType::CountryName, "GB");
    intermediate_params
        .distinguished_name
        .push(DnType::OrganizationName, "kumo-intermediate-testing");
    intermediate_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    intermediate_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    intermediate_params.use_authority_key_identifier_extension = true;
    let intermediate_key = KeyPair::generate()?;
    let intermediate_ca =
        CertifiedIssuer::signed_by(intermediate_params, intermediate_key, &root_ca)?;

    // End entity certificate for client authentication (signed by intermediate)
    let mut entity_params = CertificateParams::new(vec!["smtp.example.com".to_string()])?;
    entity_params.distinguished_name = DistinguishedName::new();
    entity_params
        .distinguished_name
        .push(DnType::CommonName, COMMON_NAME);
    entity_params.is_ca = IsCa::NoCa;
    entity_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    entity_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    entity_params.use_authority_key_identifier_extension = true;

    let entity_key = KeyPair::generate()?;
    let entity = entity_params.signed_by(&entity_key, &intermediate_ca)?;

    Ok((
        root_ca.pem(),
        intermediate_ca.pem(),
        entity.pem(),
        entity_key.serialize_pem(),
    ))
}

/// Use openssl as TLS library, confirm delivery succeeds
#[tokio::test]
async fn tls_client_certificate_rustls_openssl_success() -> anyhow::Result<()> {
    let (ca_pem, intermediate_pem, entity_pem, key_pem) = generate_certs()?;
    let cert_pem = format!("{entity_pem}\n{intermediate_pem}");

    let ex = DeliverySummary {
        source_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
        sink_counts: BTreeMap::from([(RecordType::Reception, 1), (RecordType::Delivery, 1)]),
    };

    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "true"),
        ("KUMOD_CLIENT_CERTIFICATE", &cert_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", &key_pem),
        ("KUMOD_CLIENT_REQUIRED_CA", &ca_pem),
    ];
    tls_client_certificate(env, ex).await
}

/// Use openssl as TLS library, confirm we're failing to deliver due to missing intermediate certificate in the chain.
#[tokio::test]
async fn tls_client_certificate_rustls_openssl_missing_intermediate() -> anyhow::Result<()> {
    let (ca_pem, _intermediate_pem, entity_pem, key_pem) = generate_certs()?;

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
        ("KUMOD_CLIENT_CERTIFICATE", &entity_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", &key_pem),
        ("KUMOD_CLIENT_REQUIRED_CA", &ca_pem),
    ];
    tls_client_certificate(env, ex).await
}

/// Use rustls as TLS library, confirm we're failing to deliver due to missing intermediate certificate in the chain.
#[tokio::test]
async fn tls_client_certificate_rustls_missing_intermediate() -> anyhow::Result<()> {
    let (ca_pem, _intermediate_pem, entity_pem, key_pem) = generate_certs()?;

    let ex = DeliverySummary {
        source_counts: BTreeMap::from([
            (RecordType::Reception, 1),
            (RecordType::TransientFailure, 1),
        ]),
        sink_counts: BTreeMap::new(),
    };

    let env = vec![
        ("KUMOD_ENABLE_TLS", "OpportunisticInsecure"),
        ("KUMOD_PREFER_OPENSSL", "false"),
        ("KUMOD_CLIENT_CERTIFICATE", &entity_pem),
        ("KUMOD_CLIENT_PRIVATE_KEY", &key_pem),
        ("KUMOD_CLIENT_REQUIRED_CA", &ca_pem),
    ];
    tls_client_certificate(env, ex).await
}

/// Adding fake private key to confirm openssl injection would temp fail
#[tokio::test]
async fn tls_client_certificate_rustls_openssl_fail() -> anyhow::Result<()> {
    let (_ca_pem, _intermediate_pem, cert_pem, _key_pem) = generate_certs()?;
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
    let expect_peer = env
        .iter()
        .any(|(name, _)| *name == "KUMOD_CLIENT_REQUIRED_CA");
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
        eprintln!("expect_peer={expect_peer}");
        if expect_peer {
            assert_equal!(
                reception.tls_peer_subject_name.as_ref().unwrap(),
                &vec![format!("CN={COMMON_NAME}")]
            );
        }
    }
    Ok(())
}
