use crate::queue::QueueManager;
use axum::extract::Json;
use kumo_api_types::rebind::{RebindV1Request, RebindV1Response};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::rt_spawn;
use message::message::QueueNameComponents;
use std::sync::Arc;

#[derive(Debug)]
pub struct AdminRebindEntry {
    pub request: RebindV1Request,
}

fn match_criteria(current_thing: Option<&str>, wanted_thing: Option<&str>) -> bool {
    match (current_thing, wanted_thing) {
        (Some(a), Some(b)) => a == b,
        (None, Some(_)) => {
            // Needs to match a specific thing and there is none
            false
        }
        (_, None) => {
            // No specific campaign required
            true
        }
    }
}

impl AdminRebindEntry {
    pub fn matches(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
        routing_domain: Option<&str>,
    ) -> bool {
        if !match_criteria(campaign, self.request.campaign.as_deref()) {
            return false;
        }
        if !match_criteria(tenant, self.request.tenant.as_deref()) {
            return false;
        }
        if !match_criteria(domain, self.request.domain.as_deref()) {
            return false;
        }
        if !match_criteria(routing_domain, self.request.routing_domain.as_deref()) {
            return false;
        }
        true
    }

    pub async fn list_matching_queues(&self) -> Vec<String> {
        let mut names = QueueManager::all_queue_names();
        names.retain(|queue_name| {
            let components = QueueNameComponents::parse(queue_name);
            self.matches(
                components.campaign,
                components.tenant,
                Some(components.domain),
                components.routing_domain,
            )
        });
        names
    }
}

/// Allows the system operator to administratively rebind messages.
/// Rebinding can target queues that match
/// certain criteria, or if no criteria are provided, ALL queues.
/// Rebinding is moving a message from one scheduled queue into another.
/// Queue selection is based upon the envelope recipient and message
/// metadata as described in <https://docs.kumomta.com/reference/queues/>
#[utoipa::path(
    post,
    tag="rebind",
    path="/api/admin/rebind/v1",
    responses(
        (status = 200, description = "Rebind added successfully", body=RebindV1Response)
    ),
)]
pub async fn rebind_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<RebindV1Request>,
) -> Result<Json<RebindV1Response>, AppError> {
    let entry = Arc::new(AdminRebindEntry { request });

    let queue_names = entry.list_matching_queues().await;

    // Move into a lua-capable thread so that logging related
    // lua events can be triggered by log_disposition.
    rt_spawn("process_rebind_v1".to_string(), async move {
        for name in &queue_names {
            if let Some(q) = QueueManager::get_opt(name) {
                q.rebind_all(&entry).await;
            }
        }
    })?;

    Ok(Json(RebindV1Response {}))
}
