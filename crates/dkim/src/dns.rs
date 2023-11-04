use crate::DKIMError;
use futures::future::BoxFuture;
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::TokioAsyncResolver;

/// A trait for entities that perform DNS resolution.
pub trait Lookup: Sync + Send {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DKIMError>>;
}

fn to_lookup_error(err: ResolveError) -> DKIMError {
    match err.kind() {
        ResolveErrorKind::NoRecordsFound { .. } => DKIMError::NoKeyForSignature,
        _ => DKIMError::KeyUnavailable(format!("failed to query DNS: {}", err)),
    }
}

impl Lookup for TokioAsyncResolver {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DKIMError>> {
        Box::pin(async move {
            self.txt_lookup(name)
                .await
                .map_err(to_lookup_error)?
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
