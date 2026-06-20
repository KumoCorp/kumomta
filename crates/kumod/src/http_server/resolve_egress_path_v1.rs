use crate::queue::Queue;
use crate::ready_queue::{ReadyQueueManager, GET_EGRESS_PATH_CONFIG_SIG};
use axum::extract::{Json, Query};
use kumo_api_types::egress_path::{
    EgressPathConfig, EgressPathConfigConstraints, MxResolution,
};
use kumo_api_types::{ResolveEgressPathV1Request, ResolveEgressPathV1Response};
use kumo_server_common::config_handle::ConfigHandle;
use kumo_server_common::http_server::AppError;

/// Resolve the effective egress path configuration and throughput
/// ceilings for a destination domain and egress source.
///
/// {{since('dev')}}
///
/// Mirrors the diagnostic that `kcli inspect-ready-q` provides for
/// live ready queues, but operates from the configuration side: it
/// invokes the `get_queue_config` and `get_egress_path_config` event
/// callbacks for the supplied `domain`/`source` pair, performs the
/// associated MX lookup, and returns the resulting configuration,
/// derived ceilings and the ready-queue name that would be used.
#[utoipa::path(
    get,
    tags=["inspect", "kcli:resolve-egress-path"],
    path="/api/admin/resolve-egress-path/v1",
    params(ResolveEgressPathV1Request),
    responses(
        (status = 200, description = "Resolved egress path", body=ResolveEgressPathV1Response),
    ),
)]
pub async fn resolve_v1(
    Query(request): Query<ResolveEgressPathV1Request>,
) -> Result<Json<ResolveEgressPathV1Response>, AppError> {
    let source = request.source.unwrap_or_else(|| "unspecified".to_string());

    let mut config = config::load_config().await?;
    let queue_config = Queue::call_get_queue_config(&request.domain, &mut config).await?;
    let queue_config = ConfigHandle::new(queue_config);

    let (queue_name, site_name, mx) =
        match ReadyQueueManager::compute_queue_name(&request.domain, &queue_config, &source).await
        {
            Ok(ready_name) => {
                let mx = ready_name.mx.as_ref().map(|m| MxResolution::from(&**m));
                (ready_name.name, ready_name.site_name, mx)
            }
            Err(_) => {
                // DNS resolution failed (or wasn't applicable). Fall
                // back to using `domain` as the site name so we can
                // still synthesize a queue name and look up the path
                // config, matching the behavior of
                // resolve-shaping-domain.
                let site_name = request.domain.clone();
                let proto_part = queue_config.borrow().protocol.ready_queue_name();
                let queue_name = format!("{source}->{site_name}@{proto_part}");
                (queue_name, site_name, None)
            }
        };

    let path_config: EgressPathConfig = config
        .async_call_callback(
            &GET_EGRESS_PATH_CONFIG_SIG,
            (request.domain.clone(), source.clone(), site_name),
        )
        .await?;
    config.put();

    let constraints: EgressPathConfigConstraints = path_config.compute_constraints();

    Ok(Json(ResolveEgressPathV1Response {
        domain: request.domain,
        source,
        mx,
        queue_name,
        path_config,
        constraints,
    }))
}
