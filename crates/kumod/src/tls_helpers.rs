use anyhow::Context;
use rustls::ServerConfig;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn make_server_config(
    hostname: &str,
    tls_private_key: &Option<PathBuf>,
    tls_certificate: &Option<PathBuf>,
) -> anyhow::Result<Arc<ServerConfig>> {
    let mut certificates = vec![];
    let private_key = match tls_private_key {
        Some(key) => load_private_key(key)?,
        None => {
            let cert = rcgen::generate_simple_self_signed(vec![hostname.to_string()])?;
            certificates.push(rustls::Certificate(cert.serialize_der()?));
            rustls::PrivateKey(cert.serialize_private_key_der())
        }
    };

    if let Some(cert_file) = tls_certificate {
        certificates = load_certs(cert_file)?;
    }

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)?;

    Ok(Arc::new(config))
}

fn load_certs(filename: &Path) -> anyhow::Result<Vec<rustls::Certificate>> {
    let certfile = std::fs::File::open(filename)
        .with_context(|| format!("cannot open certificate file {}", filename.display()))?;

    let mut reader = std::io::BufReader::new(certfile);
    Ok(rustls_pemfile::certs(&mut reader)
        .with_context(|| {
            format!(
                "reading PEM encoded certificates from {}",
                filename.display()
            )
        })?
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect())
}

fn load_private_key(filename: &Path) -> anyhow::Result<rustls::PrivateKey> {
    let keyfile = std::fs::File::open(filename)
        .with_context(|| format!("cannot open private key file {}", filename.display()))?;
    let mut reader = std::io::BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return Ok(rustls::PrivateKey(key)),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return Ok(rustls::PrivateKey(key)),
            Some(rustls_pemfile::Item::ECKey(key)) => return Ok(rustls::PrivateKey(key)),
            None => break,
            _ => {}
        }
    }

    anyhow::bail!(
        "no keys found in {} (encrypted keys not supported)",
        filename.display()
    );
}
