//! A reqwest client builder pinned to the OS certificate store.

/// Returns a reqwest `ClientBuilder` that trusts only the OS certificate store.
/// Callers add their own settings, then `build`. This is the one sanctioned
/// place to construct a reqwest client. A clippy `disallowed-methods` lint
/// forbids reqwest `Client::builder`/`Client::new` elsewhere.
#[allow(clippy::disallowed_methods)]
pub fn builder() -> reqwest::ClientBuilder {
    // reqwest also accepts certificates signed by a Mozilla root set compiled
    // into the binary whenever its rustls-tls-webpki-roots feature is on,
    // which a transitive dependency (vaultrs' rustify) forces on build-wide.
    // A CA pulled from the OS store would still be vouched for by that
    // compiled-in set. We disable it here, leaving the OS store as the only
    // trust source.
    reqwest::Client::builder().tls_built_in_webpki_certs(false)
}
