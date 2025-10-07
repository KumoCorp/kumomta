use crate::queue::QueueManager;
use axum::extract::Json;
use kumo_api_types::xfer::{XferV1Request, XferV1Response};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::rt_spawn;
use message::message::QueueNameComponents;
use std::sync::Arc;

#[derive(Debug)]
pub struct AdminXferEntry {
    pub request: XferV1Request,
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

impl AdminXferEntry {
    pub fn matches(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
        routing_domain: Option<&str>,
        queue_name: Option<&str>,
    ) -> bool {
        if !self.request.queue_names.is_empty() {
            if let Some(queue_name) = queue_name {
                return self
                    .request
                    .queue_names
                    .iter()
                    .any(|name| name == queue_name);
            }
            // When queue_names is set, only matching queue names
            // can possibly match
            return false;
        }
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
                Some(queue_name),
            )
        });
        names
    }
}

/// Allows the system operator to transfer messages from the current
/// node to some other target node.
/// The transfer (xfer) can target queues that match
/// certain criteria, or if no criteria are provided, ALL queues.
/// Queue selection is based upon the envelope recipient and message
/// metadata as described in <https://docs.kumomta.com/reference/queues/>.
/// Messages in the selected queues will be moved into an xfer queue
/// whose name is based on the target specified by the transfer request.
#[utoipa::path(
    post,
    tag="xfer",
    path="/api/admin/xfer/v1",
    responses(
        (status = 200, description = "Xfer added successfully", body=XferV1Response)
    ),
)]
pub async fn xfer_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<XferV1Request>,
) -> Result<Json<XferV1Response>, AppError> {
    let entry = Arc::new(AdminXferEntry { request });

    let queue_names = entry.list_matching_queues().await;

    // Move into a lua-capable thread so that logging related
    // lua events can be triggered by log_disposition.
    rt_spawn("process_xfer_v1".to_string(), async move {
        for name in &queue_names {
            if let Some(q) = QueueManager::get_opt(name) {
                q.xfer_all(&entry).await;
            }
        }
    })?;

    Ok(Json(XferV1Response {}))
}
