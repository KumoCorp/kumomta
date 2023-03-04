use anyhow::Context;
use data_loader::KeySource;
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
            let cert = rcgen::generate_simple_self_signed(vec![hostname.to_string()])?;
            certificates.push(rustls::Certificate(cert.serialize_der()?));
            rustls::PrivateKey(cert.serialize_private_key_der())
        }
    };

    if let Some(cert_file) = tls_certificate {
        let data = cert_file.get().await?;
        certificates = load_certs(&data)
            .with_context(|| format!("loading certificates from {cert_file:?}"))?;
    }

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)?;

    Ok(Arc::new(config))
}

fn load_certs(data: &[u8]) -> anyhow::Result<Vec<rustls::Certificate>> {
    let mut reader = std::io::BufReader::new(data);
    Ok(rustls_pemfile::certs(&mut reader)
        .with_context(|| format!("reading PEM encoded certificates",))?
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect())
}

fn load_private_key(data: &[u8]) -> anyhow::Result<rustls::PrivateKey> {
    let mut reader = std::io::BufReader::new(data);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return Ok(rustls::PrivateKey(key)),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return Ok(rustls::PrivateKey(key)),
            Some(rustls_pemfile::Item::ECKey(key)) => return Ok(rustls::PrivateKey(key)),
            None => break,
            _ => {}
        }
    }

    anyhow::bail!("no keys found in key data (encrypted keys not supported)",);
}
