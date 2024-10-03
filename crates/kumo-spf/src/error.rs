use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpfError {
    #[error("DNS record {0} not found")]
    DnsRecordNotFound(String),
    #[error("{0}")]
    DnsLookupFailed(String),
}
