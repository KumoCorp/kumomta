use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DnsError {
    #[error("SPF: DNS record {0} not found")]
    NotFound(String),
    #[error("SPF: {0}")]
    LookupFailed(String),
}

impl DnsError {
    pub(crate) fn from_resolve(name: &str, err: ResolveError) -> Self {
        match err.kind() {
            ResolveErrorKind::NoRecordsFound { .. } => DnsError::NotFound(name.to_string()),
            _ => DnsError::LookupFailed(format!("failed to query DNS for {name}: {err}")),
        }
    }
}
