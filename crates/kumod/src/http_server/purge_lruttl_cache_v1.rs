use axum::extract::Json;
use axum::http::StatusCode;
use kumo_api_types::{PurgeLruttlCacheV1Request, PurgeLruttlCacheV1Response};
use kumo_server_common::http_server::AppError;

/// Purge (invalidate) all entries from a single named lruttl cache, so that the
/// next lookup repopulates it from source.
///
/// This targets only the named cache, unlike a global config-epoch bump, which
/// invalidates every epoch-scoped cache at once. It lets a low-frequency cache
/// be taken off epoch invalidation and refreshed explicitly by whichever
/// component owns its backing data.
#[utoipa::path(
    post,
    tags=["config"],
    path="/api/admin/purge-lruttl-cache",
    request_body=PurgeLruttlCacheV1Request,
    responses(
        (status = 200, description = "Cache purged", body=PurgeLruttlCacheV1Response),
        (status = 404, description = "No cache is registered under that name"),
    ),
)]
pub async fn purge(
    // Note: Json<> must be last in the param list
    Json(request): Json<PurgeLruttlCacheV1Request>,
) -> Result<Json<PurgeLruttlCacheV1Response>, AppError> {
    match lruttl::purge_cache_by_name(&request.name) {
        Some(purged) => Ok(Json(PurgeLruttlCacheV1Response { purged })),
        None => Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("no lruttl cache named {} is registered", request.name),
        )),
    }
}
