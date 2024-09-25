use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpfError {
    #[error("DNS record {0} not found")]
    DnsRecordNotFound(String),
    #[error("{0}")]
    DnsLookupFailed(String),
}

impl SpfError {
    pub(crate) fn from_resolve(name: &str, err: ResolveError) -> Self {
        match err.kind() {
            ResolveErrorKind::NoRecordsFound { .. } => {
                SpfError::DnsRecordNotFound(name.to_string())
            }
            _ => SpfError::DnsLookupFailed(format!("failed to query DNS for {name}: {err}")),
        }
    }
}
