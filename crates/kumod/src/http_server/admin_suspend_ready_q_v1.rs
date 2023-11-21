use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kumo_api_types::{
    SuspendReadyQueueV1ListEntry, SuspendReadyQueueV1Request, SuspendV1CancelRequest,
    SuspendV1Response,
};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;

use std::sync::Mutex;
use std::time::Instant;
use uuid::Uuid;

lazy_static::lazy_static! {
    static ref ENTRIES: Mutex<Vec<AdminSuspendReadyQEntry>> = Mutex::new(vec![]);
}

#[derive(Clone, Debug)]
pub struct AdminSuspendReadyQEntry {
    pub id: Uuid,
    pub name: String,
    pub reason: String,
    pub expires: Instant,
}

impl AdminSuspendReadyQEntry {
    pub fn get_duration(&self) -> std::time::Duration {
        self.expires.saturating_duration_since(Instant::now())
    }
    pub fn get_duration_chrono(&self) -> chrono::Duration {
        chrono::Duration::from_std(self.get_duration())
            .unwrap_or_else(|_| chrono::Duration::seconds(60))
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

impl AdminSuspendReadyQEntry {
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
        entries.retain(|ent| ent.expires > now && ent.name != entry.name);

        entries.push(entry);
    }

    pub fn matches(&self, name: Option<&str>) -> bool {
        match_criteria(name, Some(&self.name))
    }

    pub fn get_matching(name: Option<&str>) -> Vec<Self> {
        let mut entries = Self::get_all();
        entries.retain(|ent| ent.matches(name));
        entries
    }

    pub fn get_for_queue_name(name: &str) -> Option<Self> {
        let mut entries = Self::get_matching(Some(name));
        entries.pop()
    }
}

/// Define a suspension for a ready queue
#[utoipa::path(
    post,
    tag="suspend",
    path="/api/admin/suspend-ready-q/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendV1Response),
    ),
)]
pub async fn suspend(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<SuspendReadyQueueV1Request>,
) -> Result<Json<SuspendV1Response>, AppError> {
    let duration = request.duration();
    let entry = AdminSuspendReadyQEntry {
        id: Uuid::new_v4(),
        name: request.name,
        reason: request.reason,
        expires: Instant::now() + duration,
    };

    AdminSuspendReadyQEntry::add(entry.clone());

    Ok(Json(SuspendV1Response { id: entry.id }))
}

/// List the active ready-queue suspensions
#[utoipa::path(
    get,
    tag="suspend",
    path="/api/admin/suspend-ready-q/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendReadyQueueV1ListEntry),
    ),
)]
pub async fn list(
    _: TrustedIpRequired,
) -> Result<Json<Vec<SuspendReadyQueueV1ListEntry>>, AppError> {
    let now = Instant::now();
    Ok(Json(
        AdminSuspendReadyQEntry::get_all()
            .into_iter()
            .filter_map(|entry| {
                entry.expires.checked_duration_since(now).map(|duration| {
                    SuspendReadyQueueV1ListEntry {
                        id: entry.id,
                        name: entry.name,
                        reason: entry.reason,
                        duration,
                    }
                })
            })
            .collect(),
    ))
}

/// Remove a ready-queue suspension
#[utoipa::path(
    delete,
    tag="suspend",
    path="/api/admin/suspend-ready-q/v1",
    responses(
        (status = 200, description = "Removed the suspension"),
        (status = 404, description = "Suspension either expired or was never valid"),
    ),
)]
pub async fn delete(_: TrustedIpRequired, Json(request): Json<SuspendV1CancelRequest>) -> Response {
    let removed = AdminSuspendReadyQEntry::remove_by_id(&request.id);
    if removed {
        (StatusCode::OK, format!("removed {}", request.id))
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("suspend-ready-q entry {} not found", request.id),
        )
    }
    .into_response()
}
