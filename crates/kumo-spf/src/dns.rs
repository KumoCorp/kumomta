use crate::error::SpfError;
use futures::future::BoxFuture;
use hickory_resolver::error::ResolveErrorKind;
use hickory_resolver::TokioAsyncResolver;

/// A trait for entities that perform DNS resolution.
pub trait Lookup: Sync + Send {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, SpfError>>;
}

impl Lookup for TokioAsyncResolver {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, SpfError>> {
        Box::pin(async move {
            self.txt_lookup(name)
                .await
                .map_err(|err| match err.kind() {
                    ResolveErrorKind::NoRecordsFound { .. } => {
                        SpfError::DnsRecordNotFound(name.to_string())
                    }
                    _ => {
                        SpfError::DnsLookupFailed(format!("failed to query DNS for {name}: {err}"))
                    }
                })?
                .into_iter()
                .map(|txt| {
                    Ok(txt
                        .iter()
                        .map(|data| String::from_utf8_lossy(data))
                        .collect())
                })
                .collect()
        })
    }
}
