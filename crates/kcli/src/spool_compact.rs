use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::SpoolCompactV1Request;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Forces a flush and full compaction of the named spool.
///
/// Primarily a diagnostic and test helper.  For rocksdb-backed spools
/// this calls flush() followed by a full-keyspace compact_range().
/// For other spool kinds it is a no-op.
///
/// If the underlying storage reports an error during the operation
/// (for example, a missing or corrupt SST file in a rocksdb spool),
/// the error is reported to the caller and the command exits non-zero.
pub struct SpoolCompactCommand {
    /// Name of the spool to compact, matching a name passed to
    /// `kumo.define_spool` in the policy.
    #[arg(long)]
    name: String,
}

impl SpoolCompactCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        KumoApiClient::new(endpoint.clone())
            .admin_spool_compact_v1(&SpoolCompactV1Request {
                name: self.name.clone(),
            })
            .await?;
        Ok(())
    }
}
