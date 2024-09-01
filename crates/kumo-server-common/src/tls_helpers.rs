use anyhow::Context;
use data_loader::KeySource;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig;
use std::sync::Arc;

pub async fn make_server_config(
    hostname: &str,
    tls_private_key: &Option<KeySource>,
    tls_certificate: &Option<KeySource>,
) -> anyhow::Result<Arc<ServerConfig>> {
    let mut certificates = vec![];
    let private_key = match tls_private_key {
        Some(key) => {
            let data = key.get().await?;
            load_private_key(&data).with_context(|| format!("loading private key from {key:?}"))?
        }
        None => {
            let key = rcgen::generate_simple_self_signed(vec![hostname.to_string()])?;
            certificates.push(CertificateDer::from_slice(key.cert.der()).into_owned());
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.key_pair.serialize_der()))
        }
    };

    if let Some(cert_file) = tls_certificate {
        let data = cert_file.get().await?;
        certificates = load_certs(&data)
            .with_context(|| format!("loading certificates from {cert_file:?}"))?;
    }

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)?;

    Ok(Arc::new(config))
}

fn load_certs(data: &[u8]) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let mut certs = vec![];
    let mut reader = std::io::BufReader::new(data);
    for res in rustls_pemfile::certs(&mut reader) {
        let cert = res.context("failed to read PEM encoded certificates")?;
        certs.push(cert);
    }
    Ok(certs)
}

fn load_private_key(data: &[u8]) -> anyhow::Result<PrivateKeyDer<'static>> {
    let mut reader = std::io::BufReader::new(data);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::Pkcs8Key(key)) => return Ok(key.into()),
            Some(rustls_pemfile::Item::Pkcs1Key(key)) => return Ok(key.into()),
            Some(rustls_pemfile::Item::Sec1Key(key)) => return Ok(key.into()),
            None => break,
            _ => {}
        }
    }

    anyhow::bail!("no keys found in key data (encrypted keys not supported)",);
}
