use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kumo_api_types::{
    SuspendV1CancelRequest, SuspendV1ListEntry, SuspendV1Request, SuspendV1Response,
};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use message::message::QueueNameComponents;
use std::sync::Mutex;
use std::time::Instant;
use uuid::Uuid;

lazy_static::lazy_static! {
    static ref ENTRIES: Mutex<Vec<AdminSuspendEntry>> = Mutex::new(vec![]);
}

#[derive(Clone, Debug)]
pub struct AdminSuspendEntry {
    pub id: Uuid,
    pub campaign: Option<String>,
    pub tenant: Option<String>,
    pub domain: Option<String>,
    pub reason: String,
    pub expires: Instant,
}

impl AdminSuspendEntry {
    pub fn get_duration(&self) -> chrono::Duration {
        let duration = self.expires.saturating_duration_since(Instant::now());
        chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::seconds(1))
    }
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

impl AdminSuspendEntry {
    pub fn get_all() -> Vec<Self> {
        let mut entries = ENTRIES.lock().unwrap();
        let now = Instant::now();
        entries.retain(|ent| ent.expires > now);
        entries.clone()
    }

    pub fn remove_by_id(id: &Uuid) -> bool {
        let mut entries = ENTRIES.lock().unwrap();
        let len_before = entries.len();
        entries.retain(|e| e.id != *id);
        len_before != entries.len()
    }

    pub fn add(entry: Self) {
        let mut entries = ENTRIES.lock().unwrap();
        let now = Instant::now();
        // Age out expired entries, and replace any entries with the
        // same criteria; this allows updating the reason with a newer
        // version of the suspend info.
        entries.retain(|ent| {
            ent.expires > now
                && !(ent.campaign == entry.campaign
                    && ent.tenant == entry.tenant
                    && ent.domain == entry.domain)
        });

        entries.push(entry);
    }

    pub fn matches(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
    ) -> bool {
        if !match_criteria(campaign, self.campaign.as_deref()) {
            return false;
        }
        if !match_criteria(tenant, self.tenant.as_deref()) {
            return false;
        }
        if !match_criteria(domain, self.domain.as_deref()) {
            return false;
        }
        true
    }

    pub fn get_matching(
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
    ) -> Vec<Self> {
        let mut entries = Self::get_all();
        entries.retain(|ent| ent.matches(campaign, tenant, domain));
        entries
    }

    pub fn get_for_queue_name(queue_name: &str) -> Option<Self> {
        let components = QueueNameComponents::parse(queue_name);
        let mut entries = Self::get_matching(
            components.campaign,
            components.tenant,
            Some(components.domain),
        );
        entries.pop()
    }
}

/// Define a suspension for a scheduled queue
#[utoipa::path(
    post,
    tag="suspend",
    path="/api/admin/suspend/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendV1Response),
    ),
)]
pub async fn suspend(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<SuspendV1Request>,
) -> Result<Json<SuspendV1Response>, AppError> {
    let duration = request.duration();
    let entry = AdminSuspendEntry {
        id: Uuid::new_v4(),
        campaign: request.campaign,
        tenant: request.tenant,
        domain: request.domain,
        reason: request.reason,
        expires: Instant::now() + duration,
    };

    AdminSuspendEntry::add(entry.clone());

    Ok(Json(SuspendV1Response { id: entry.id }))
}

/// List the active scheduled-queue suspensions
#[utoipa::path(
    get,
    tag="suspend",
    path="/api/admin/suspend/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendV1ListEntry),
    ),
)]
pub async fn list(_: TrustedIpRequired) -> Result<Json<Vec<SuspendV1ListEntry>>, AppError> {
    let now = Instant::now();
    Ok(Json(
        AdminSuspendEntry::get_all()
            .into_iter()
            .filter_map(|entry| {
                entry
                    .expires
                    .checked_duration_since(now)
                    .map(|duration| SuspendV1ListEntry {
                        id: entry.id,
                        campaign: entry.campaign,
                        tenant: entry.tenant,
                        domain: entry.domain,
                        reason: entry.reason,
                        duration,
                    })
            })
            .collect(),
    ))
}

/// Remove a scheduled-queue suspension
#[utoipa::path(
    delete,
    tag="suspend",
    path="/api/admin/suspend/v1",
    responses(
        (status = 200, description = "Removed the suspension"),
        (status = 404, description = "Suspension either expired or was never valid"),
    ),
)]
pub async fn delete(_: TrustedIpRequired, Json(request): Json<SuspendV1CancelRequest>) -> Response {
    let removed = AdminSuspendEntry::remove_by_id(&request.id);
    if removed {
        (StatusCode::OK, format!("removed {}", request.id))
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("suspend entry {} not found", request.id),
        )
    }
    .into_response()
}
