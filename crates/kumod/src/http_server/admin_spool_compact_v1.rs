use crate::spool::SpoolManager;
use axum::extract::Json;
use kumo_api_types::SpoolCompactV1Request;
use kumo_server_common::http_server::AppError;

/// Force a flush and major compaction of the named spool.
///
/// Primarily intended for operational diagnostics and integration
/// tests.  For rocksdb-backed spools this performs a flush of the
/// memtable followed by a full-keyspace compaction.  For other spool
/// kinds it is a no-op.
///
/// Returns an error if the named spool is not defined, or if the
/// underlying storage reports an error (for example, a missing or
/// corrupt SST file encountered during compaction).
#[utoipa::path(
    post,
    tags=["spool", "kcli:spool-compact"],
    path="/api/admin/spool-compact/v1",
    request_body=SpoolCompactV1Request,
    responses(
        (status = 200, description = "Compaction completed successfully"),
    ),
)]
pub async fn spool_compact_v1(Json(request): Json<SpoolCompactV1Request>) -> Result<(), AppError> {
    let spool = SpoolManager::get_named(&request.name).await?;
    spool.compact().await?;
    Ok(())
}
