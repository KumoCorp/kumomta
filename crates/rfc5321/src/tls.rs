#![cfg(feature = "client")]
use hickory_proto::rr::rdata::TLSA;
use openssl::ssl::SslOptions;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::certs;
use std::io::BufReader;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tokio_rustls::rustls::client::danger::ServerCertVerifier;
use tokio_rustls::rustls::crypto::{aws_lc_rs as provider, CryptoProvider};
use tokio_rustls::rustls::{ClientConfig, SupportedCipherSuite};
use tokio_rustls::TlsConnector;

#[derive(Clone, Debug)]
struct RustlsCacheKey {
    insecure: bool,
    rustls_cipher_suites: Vec<SupportedCipherSuite>,
}

// SupportedCipherSuite has a PartialEq impl but not an Eq impl.
// Since we need RustlsCacheKey to be Hash we cannot simply derive
// PartialEq and then add an explicit impl for Eq on RustlsCacheKey
// because we don't know the implementation details of the underlying
// PartialEq impl. So we define our own here where we explicitly compare
// the suite names. This may not be strictly necessary, but it seems
// wise to be robust to possible future weirdness in that type, and
// to be certain that our Hash impl is consistent with the Eq impl.
impl std::cmp::PartialEq for RustlsCacheKey {
    fn eq(&self, other: &RustlsCacheKey) -> bool {
        if self.insecure != other.insecure {
            return false;
        }
        self.rustls_cipher_suites
            .iter()
            .map(|s| s.suite())
            .eq(other.rustls_cipher_suites.iter().map(|s| s.suite()))
    }
}

impl std::cmp::Eq for RustlsCacheKey {}

impl std::hash::Hash for RustlsCacheKey {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: std::hash::Hasher,
    {
        self.insecure.hash(hasher);
        for suite in &self.rustls_cipher_suites {
            suite.suite().as_str().hash(hasher);
        }
    }
}

lruttl::declare_cache! {
static RUSTLS_CACHE: LruCacheWithTtl<RustlsCacheKey, Arc<ClientConfig>>::new("rfc5321_rustls_config", 32);
}

impl RustlsCacheKey {
    fn get(&self) -> Option<Arc<ClientConfig>> {
        RUSTLS_CACHE.get(self)
    }

    async fn set(self, value: Arc<ClientConfig>) {
        RUSTLS_CACHE
            .insert(
                self,
                value,
                // We allow the state to be cached for up to 15 minutes at
                // a time so that we have an opportunity to reload the
                // system certificates within a reasonable time frame
                // as/when they are updated by the system.
                Instant::now() + Duration::from_secs(15 * 60),
            )
            .await;
    }
}

#[derive(Debug, Clone, Default)]
pub struct TlsOptions {
    pub insecure: bool,
    pub alt_name: Option<String>,
    pub dane_tlsa: Vec<TLSA>,
    pub prefer_openssl: bool,
    pub certificate_from_pem: Option<Vec<u8>>,
    pub private_key_from_pem: Option<Vec<u8>>,
    pub openssl_cipher_list: Option<String>,
    pub openssl_cipher_suites: Option<String>,
    pub openssl_options: Option<SslOptions>,
    pub rustls_cipher_suites: Vec<SupportedCipherSuite>,
}

impl TlsOptions {
    /// Produce a TlsConnector for this set of TlsOptions.
    /// We need to employ a cache around the verifier as loading
    /// the system certificate store can be a non-trivial operation
    /// and not be something we want to do repeatedly in a hot code
    /// path.  The cache does unfortunately complicate some of the
    /// internals here.
    pub async fn build_tls_connector(&self) -> TlsConnector {
        let key = RustlsCacheKey {
            insecure: self.insecure,
            rustls_cipher_suites: self.rustls_cipher_suites.clone(),
        };
        if let Some(config) = key.get() {
            return TlsConnector::from(config);
        }
        let cipher_suites = if self.rustls_cipher_suites.is_empty() {
            provider::DEFAULT_CIPHER_SUITES
        } else {
            &self.rustls_cipher_suites
        };

        let provider = Arc::new(CryptoProvider {
            cipher_suites: cipher_suites.to_vec(),
            ..provider::default_provider()
        });

        let verifier: Arc<dyn ServerCertVerifier> = if self.insecure {
            Arc::new(danger::NoCertificateVerification::new(provider.clone()))
        } else {
            Arc::new(rustls_platform_verifier::Verifier::new().with_provider(provider.clone()))
        };

        let mut rustls_certificate: Option<Arc<Vec<CertificateDer<'static>>>> = None;
        let mut rustls_private_key: Option<Arc<PrivateKeyDer<'static>>> = None;

        match &self.private_key_from_pem {
            None => {}
            Some(pem) => match self.load_private_key(&pem).await {
                Ok(key) => {
                    rustls_private_key = Some(Arc::new(key));
                }
                Err(err) => {
                    tracing::error!("failed to load private key: {err:#}");
                }
            },
        }

        match &self.certificate_from_pem {
            None => {}
            Some(pem) => match self.load_tls_cert(&pem).await {
                Ok(cert) => {
                    rustls_certificate = Some(Arc::new(cert));
                }
                Err(err) => {
                    tracing::error!("failed to load certificate: {err:#}");
                }
            },
        }

        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(tokio_rustls::rustls::DEFAULT_VERSIONS)
            .expect("inconsistent cipher-suite/versions selected")
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone());
        let config = match (&rustls_certificate, &rustls_private_key) {
            (Some(certs), Some(key)) => match builder
                .clone()
                .with_client_auth_cert(certs.as_ref().clone(), key.as_ref().clone_key())
            {
                Ok(cfg) => Arc::new(cfg),
                Err(err) => {
                    tracing::error!("invalid client side certificate: {err:#}");
                    Arc::new(builder.with_no_client_auth())
                }
            },
            _ => Arc::new(builder.with_no_client_auth()),
        };
        key.set(config.clone()).await;

        TlsConnector::from(config)
    }

    async fn load_tls_cert(&self, data: &Vec<u8>) -> std::io::Result<Vec<CertificateDer<'static>>> {
        let mut reader = BufReader::new(data.as_slice());
        let certs = certs(&mut reader)
            .into_iter()
            .map(|r| r.map(CertificateDer::into_owned))
            .collect::<Result<Vec<CertificateDer<'static>>, std::io::Error>>()?;
        Ok(certs)
    }

    async fn load_private_key(&self, data: &Vec<u8>) -> std::io::Result<PrivateKeyDer<'static>> {
        // Try to parse as PKCS#8
        let pkcs8_keys: Vec<PrivateKeyDer<'static>> = {
            let mut reader = BufReader::new(data.as_slice());
            rustls_pemfile::pkcs8_private_keys(&mut reader)
                .into_iter()
                .map(|r| r.map(PrivateKeyDer::Pkcs8))
                .collect::<Result<Vec<PrivateKeyDer<'static>>, std::io::Error>>()?
        };

        if !pkcs8_keys.is_empty() {
            return Ok(pkcs8_keys.into_iter().next().unwrap());
        }

        // Reset reader and try as RSA PKCS#1
        let rsa_keys: Vec<PrivateKeyDer<'static>> = {
            let mut reader = BufReader::new(data.as_slice());
            rustls_pemfile::rsa_private_keys(&mut reader)
                .into_iter()
                .map(|r| r.map(PrivateKeyDer::Pkcs1))
                .collect::<Result<Vec<PrivateKeyDer<'static>>, std::io::Error>>()?
        };

        if !rsa_keys.is_empty() {
            return Ok(rsa_keys.into_iter().next().unwrap());
        }

        // Reset reader and try as EC Sec1
        let ec_keys: Vec<PrivateKeyDer<'static>> = {
            let mut reader = BufReader::new(data.as_slice());
            rustls_pemfile::ec_private_keys(&mut reader)
                .into_iter()
                .map(|r| r.map(PrivateKeyDer::Sec1))
                .collect::<Result<Vec<PrivateKeyDer<'static>>, std::io::Error>>()?
        };

        if !ec_keys.is_empty() {
            return Ok(ec_keys.into_iter().next().unwrap());
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "No private key found in PEM file",
        ))
    }
}

mod danger {
    use std::sync::Arc;
    use tokio_rustls::rustls::client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    };
    use tokio_rustls::rustls::crypto::{
        verify_tls12_signature, verify_tls13_signature, CryptoProvider,
    };
    use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use tokio_rustls::rustls::DigitallySignedStruct;

    #[derive(Debug)]
    pub struct NoCertificateVerification(Arc<CryptoProvider>);

    impl NoCertificateVerification {
        pub fn new(provider: Arc<CryptoProvider>) -> Self {
            Self(provider)
        }
    }

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }
}
