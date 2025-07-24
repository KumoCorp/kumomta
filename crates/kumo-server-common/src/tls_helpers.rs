use anyhow::Context;
use data_loader::KeySource;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use std::sync::Arc;

fn read_trust_anchor(trust_anchor: &[u8]) -> anyhow::Result<RootCertStore> {
    let mut store = RootCertStore::empty();
    let mut added = 0;
    for cert in CertificateDer::pem_slice_iter(trust_anchor) {
        store.add(cert?)?;
        added += 1;
    }

    if added == 0 {
        anyhow::bail!("failed to parse certs");
    }

    Ok(store)
}

pub async fn make_server_config(
    hostname: &str,
    tls_private_key: &Option<KeySource>,
    tls_certificate: &Option<KeySource>,
    required_client_ca: &Option<KeySource>,
) -> anyhow::Result<Arc<ServerConfig>> {
    let mut certificates = vec![];
    let private_key = match tls_private_key {
        Some(key) => PrivateKeyDer::from_pem_slice(&key.get().await?)
            .with_context(|| format!("loading private key from {key:?}"))?,
        None => {
            let key = rcgen::generate_simple_self_signed(vec![hostname.to_string()])?;
            certificates.push(CertificateDer::from_slice(key.cert.der()).into_owned());
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.key_pair.serialize_der()))
        }
    };

    if let Some(cert_file) = tls_certificate {
        let data = cert_file.get().await?;
        certificates = CertificateDer::pem_slice_iter(&data)
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("loading certificates from {cert_file:?}"))?;
    }

    let config = ServerConfig::builder();
    let config = match required_client_ca {
        Some(client_ca) => {
            let ca = client_ca.get().await?;
            let verifier = WebPkiClientVerifier::builder(read_trust_anchor(&ca)?.into()).build()?;
            config.with_client_cert_verifier(verifier)
        }
        None => config.with_no_client_auth(),
    };
    let config = config.with_single_cert(certificates, private_key)?;

    Ok(Arc::new(config))
}
